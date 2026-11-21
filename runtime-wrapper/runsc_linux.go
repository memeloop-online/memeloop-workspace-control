package main

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"syscall"
)

// Only the exact shim and dedicated TOML options are supported. The original
// arguments are forwarded unchanged; boot/gofer are invoked by real runsc, not
// synthesized or proxied by this entry.
func parseRunscArgs(args []string) (invocation, error) {
	var in invocation
	values := map[string]bool{
		"--root": true, "--log": true, "--log-format": true, "--panic-log": true,
		"--debug-log": true, "--debug-log-fd": true, "--systemd-cgroup": true,
		"--platform": true, "--overlay2": true, "--file-access": true,
		"--file-access-mounts": true, "--allow-flag-override": true, "--oci-seccomp": true, "--gvisor-marker-file": true,
	}
	seen := make(map[string]string)
	i := 0
	for i < len(args) && strings.HasPrefix(args[i], "-") {
		key, value, eq := strings.Cut(args[i], "=")
		if _, exists := seen[key]; exists {
			return in, fmt.Errorf("duplicate runsc global option %s", key)
		}
		if key == "--version" || key == "--help" {
			if len(args) != 1 || eq {
				return in, errors.New("unsupported runsc version/help arguments")
			}
			in.Command = "version"
			return in, nil
		}
		if key == "--debug" {
			if eq && value != "true" && value != "false" {
				return in, errors.New("invalid runsc debug value")
			}
		} else {
			if !values[key] {
				return in, fmt.Errorf("unsupported runsc global option %s", key)
			}
			if !eq {
				i++
				if i >= len(args) {
					return in, errors.New("missing runsc global value")
				}
				value = args[i]
			}
			if value == "" || strings.HasPrefix(value, "--") {
				return in, errors.New("empty or missing runsc global value")
			}
		}
		switch key {
		case "--overlay2":
			if value != "none" {
				return in, errors.New("runsc overlay2 must be none")
			}
		case "--file-access", "--file-access-mounts":
			if value != "shared" {
				return in, errors.New("runsc file access must be shared")
			}
		case "--allow-flag-override":
			if value != "false" {
				return in, errors.New("runsc flag override is unsupported")
			}
		case "--oci-seccomp", "--gvisor-marker-file":
			if value != "true" {
				return in, errors.New("runsc seccomp/identity marker must remain enabled")
			}
		case "--platform":
			if value != "systrap" {
				return in, errors.New("prototype requires runsc systrap platform")
			}
		case "--systemd-cgroup":
			if value != "false" && value != "true" {
				return in, errors.New("invalid runsc cgroup mode")
			}
		}
		seen[key] = value
		i++
	}
	if i == len(args) {
		return in, errors.New("explicit runsc command required")
	}
	in.Command = args[i]
	i++
	switch in.Command {
	case "create":
		if seen["--overlay2"] != "none" || seen["--file-access"] != "shared" || seen["--file-access-mounts"] != "shared" {
			return in, errors.New("runsc create requires explicit none/shared/shared storage policy")
		}
	case "delete":
	case "state", "start", "exec", "kill", "wait", "ps", "events", "list", "pause", "resume", "update", "version", "restore", "checkpoint", "run":
		return in, nil
	default:
		return in, fmt.Errorf("unsupported runsc command %s", in.Command)
	}
	options := make(map[string]bool)
	for ; i < len(args); i++ {
		a := args[i]
		if !strings.HasPrefix(a, "-") {
			if in.ID != "" {
				return in, errors.New("runsc requires exactly one container ID")
			}
			in.ID = a
			continue
		}
		key, value, eq := strings.Cut(a, "=")
		if options[key] {
			return in, fmt.Errorf("duplicate runsc command option %s", key)
		}
		options[key] = true
		if in.Command == "delete" {
			if key != "--force" || (eq && value != "true" && value != "false") {
				return in, fmt.Errorf("unsupported runsc delete option %s", key)
			}
			continue
		}
		if key != "--bundle" && key != "--pid-file" && key != "--console-socket" && key != "--user-log" {
			return in, fmt.Errorf("unsupported runsc create option %s", key)
		}
		if !eq {
			i++
			if i >= len(args) {
				return in, errors.New("missing runsc create option value")
			}
			value = args[i]
		}
		if value == "" || strings.HasPrefix(value, "--") {
			return in, errors.New("empty or missing runsc command value")
		}
		if key == "--bundle" {
			in.Bundle = value
		}
	}
	return in, nil
}

func validateRunscSpec(c config, s spec) error {
	// The exact shim synthesizes these four hints for existing shared-memory
	// semantics, including the size of its actual host tmpfs. Never accept a
	// caller-selected size, a disk source, or the previous unbounded hint.
	const shmPrefix = "dev.gvisor.spec.mount.gvisorinternaldevshm."
	shmFields := 0
	for key := range s.Annotations {
		if strings.HasPrefix(key, shmPrefix) {
			shmFields++
		}
	}
	if shmFields != 0 {
		id := s.Annotations["io.kubernetes.cri.sandbox-id"]
		if s.Annotations["io.kubernetes.cri.container-type"] != "sandbox" || !idRE.MatchString(id) || shmFields != 4 {
			return errors.New("unsupported runsc shared-memory hints")
		}
		source := filepath.Join(c.CRIStateRoot, "sandboxes", id, "shm")
		size, err := runscShmSize(source)
		if err != nil {
			return err
		}
		if err := validateRunscShmHints(c, s, size); err != nil {
			return err
		}
	}
	for key := range s.Annotations {
		if strings.HasPrefix(key, shmPrefix) && shmFields != 0 {
			continue
		}
		for _, prefix := range []string{"dev.gvisor.spec.rootfs.", "dev.gvisor.spec.mount.", "dev.gvisor.flag.", "dev.gvisor.bundle."} {
			if strings.HasPrefix(key, prefix) {
				return fmt.Errorf("unsupported runsc override annotation %s", key)
			}
		}
	}
	return nil
}

// Only this fixed CRI leaf may be writable: containerd creates shm as tmpfs
// mode 1777. Its parent remains protected and every open is no-follow.
func runscShmSize(source string) (uint64, error) {
	parent, err := openTrusted(filepath.Dir(source), true)
	if err != nil {
		return 0, err
	}
	defer parent.Close()
	fd, err := syscall.Openat(int(parent.Fd()), "shm", syscall.O_RDONLY|syscall.O_DIRECTORY|syscall.O_NOFOLLOW|syscall.O_CLOEXEC, 0)
	if err != nil {
		return 0, err
	}
	f := os.NewFile(uintptr(fd), source)
	defer f.Close()
	var st syscall.Stat_t
	if err := syscall.Fstat(fd, &st); err != nil || st.Uid != 0 {
		return 0, errors.New("untrusted runsc shared-memory root")
	}
	all, err := mounts()
	if err != nil {
		return 0, err
	}
	m, err := atMount(all, source)
	if err != nil || m.FS != "tmpfs" || m.Root != "/" || m.Device != device(st.Dev) {
		return 0, errors.New("runsc shared-memory source is not its actual tmpfs mount")
	}
	var fs syscall.Statfs_t
	if err := syscall.Fstatfs(fd, &fs); err != nil {
		return 0, err
	}
	current, err := openPath(source, true)
	if err != nil {
		return 0, err
	}
	defer current.Close()
	var now syscall.Stat_t
	if err := syscall.Fstat(int(current.Fd()), &now); err != nil || now.Dev != st.Dev || now.Ino != st.Ino {
		return 0, errors.New("runsc shared-memory directory changed")
	}
	// Match both the held capacity FD and the current path to this exact
	// mount ID; an overmount of the same tmpfs device/inode is insufficient.
	for _, root := range []*os.File{f, current} {
		info, err := os.ReadFile(fmt.Sprintf("/proc/self/fdinfo/%d", root.Fd()))
		if err != nil {
			return 0, err
		}
		id := 0
		for _, line := range strings.Split(string(info), "\n") {
			if strings.HasPrefix(line, "mnt_id:") {
				if _, err = fmt.Sscanf(line, "mnt_id: %d", &id); err != nil {
					return 0, err
				}
			}
		}
		if id == 0 || id != m.ID {
			return 0, errors.New("runsc shared-memory mount identity changed")
		}
	}
	return runscShmBytes(fs)
}

func runscShmBytes(fs syscall.Statfs_t) (uint64, error) {
	const tmpfsMagic = 0x01021994
	blockSize := fs.Frsize
	if blockSize == 0 {
		blockSize = fs.Bsize
	}
	if fs.Type != tmpfsMagic || blockSize <= 0 || fs.Blocks == 0 || fs.Blocks > ^uint64(0)/uint64(blockSize) {
		return 0, errors.New("runsc shared-memory tmpfs must have a finite size")
	}
	return fs.Blocks * uint64(blockSize), nil
}

func validateRunscShmHints(c config, s spec, size uint64) error {
	const prefix = "dev.gvisor.spec.mount.gvisorinternaldevshm."
	id := s.Annotations["io.kubernetes.cri.sandbox-id"]
	if size == 0 || !idRE.MatchString(id) || s.Annotations["io.kubernetes.cri.container-type"] != "sandbox" {
		return errors.New("unsupported runsc shared-memory hints")
	}
	source := filepath.Join(c.CRIStateRoot, "sandboxes", id, "shm")
	expected := map[string]string{prefix + "source": source, prefix + "type": "tmpfs", prefix + "share": "pod", prefix + "options": "rw,size=" + strconv.FormatUint(size, 10)}
	fields := 0
	for key, value := range s.Annotations {
		if strings.HasPrefix(key, prefix) {
			fields++
			if want, ok := expected[key]; !ok || value != want {
				return fmt.Errorf("unsupported runsc shared-memory hint %s", key)
			}
		}
	}
	if fields != len(expected) {
		return errors.New("unsupported runsc shared-memory hints")
	}
	count := 0
	for _, m := range s.Mounts {
		if m.Destination == "/dev/shm" {
			count++
			if m.Type != "tmpfs" || m.Source != source {
				return errors.New("runsc shared-memory hints differ from tmpfs mount")
			}
		}
	}
	if count != 1 {
		return errors.New("runsc requires exactly one shared-memory tmpfs")
	}
	return nil
}
