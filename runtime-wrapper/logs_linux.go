package main

import (
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"syscall"
)

type podLogs struct {
	Path   string `json:"path"`
	Target string `json:"target"`
}

// Only canonical kubelet logs may use a mounted leaf to pin the boundary
// below root-owned /var/log. Other trusted paths keep the strict policy.
func openPodLogRoot(path string) (*os.File, error) {
	f, strictErr := openTrusted(path, true)
	if strictErr == nil {
		return f, nil
	}
	if path != "/var/log/pods" {
		return nil, strictErr
	}
	ancestor, err := openTrusted("/var", true)
	if err != nil {
		return nil, err
	}
	ancestor.Close()
	parent, err := openPath("/var/log", true)
	if err != nil {
		return nil, err
	}
	var st syscall.Stat_t
	err = syscall.Fstat(int(parent.Fd()), &st)
	parent.Close()
	if err != nil || st.Uid != 0 || st.Mode&0002 != 0 {
		return nil, errors.New("untrusted canonical log parent")
	}
	f, err = openPath(path, true)
	if err != nil {
		return nil, err
	}
	fail := func(err error) (*os.File, error) {
		f.Close()
		return nil, err
	}
	if err = syscall.Fstat(int(f.Fd()), &st); err != nil {
		return fail(err)
	}
	if st.Uid != 0 || st.Mode&0022 != 0 {
		return fail(errors.New("untrusted canonical log root"))
	}
	all, err := mounts()
	if err != nil {
		return fail(err)
	}
	m, err := atMount(all, path)
	if err != nil {
		return fail(err)
	}
	// fdinfo ties the held inode to this exact mount, not a covered directory.
	info, err := os.ReadFile(fmt.Sprintf("/proc/self/fdinfo/%d", f.Fd()))
	if err != nil {
		return fail(err)
	}
	id := 0
	for _, line := range strings.Split(string(info), "\n") {
		if strings.HasPrefix(line, "mnt_id:") {
			if _, err = fmt.Sscanf(line, "mnt_id: %d", &id); err != nil {
				return fail(err)
			}
		}
	}
	if id != m.ID || id == 0 || device(st.Dev) != m.Device {
		return fail(errors.New("canonical log root mount identity changed"))
	}
	current, err := openPath(path, true)
	if err != nil {
		return fail(err)
	}
	var now syscall.Stat_t
	err = syscall.Fstat(int(current.Fd()), &now)
	current.Close()
	if err != nil || now.Dev != st.Dev || now.Ino != st.Ino {
		return fail(errors.New("canonical log root directory changed"))
	}
	return f, nil
}

// Runs only before pause create, under the existing Pod lock. Ordinary container
// loggers may already be open by their runtime create, so that point is too late.
// No delete path unlinks this shared Pod link or its target.
func redirectPodLogs(c config, sc scratch, s spec) (*podLogs, error) {
	pod := s.Annotations["io.kubernetes.cri.sandbox-uid"]
	name := s.Annotations["io.kubernetes.cri.sandbox-name"]
	if len(name) > 253 || !regexp.MustCompile(`^[a-z0-9]([-a-z0-9.]*[a-z0-9])?$`).MatchString(name) {
		return nil, errors.New("unsupported CRI Pod name")
	}
	path := filepath.Join(c.PodLogRoot, c.WorkspaceNamespace+"_"+name+"_"+pod)
	if s.Annotations["io.kubernetes.cri.sandbox-log-directory"] != path {
		return nil, errors.New("Pod log path does not match trusted namespace/name/UID")
	}
	parent, err := openPodLogRoot(c.PodLogRoot)
	if err != nil {
		return nil, err
	}
	defer parent.Close()
	targetRoot := filepath.Join(sc.Path, "runtime-private", "logs")
	target := filepath.Join(targetRoot, pod)
	for _, dir := range []string{targetRoot, target} {
		if err = os.Mkdir(dir, 0755); err != nil && !os.IsExist(err) {
			return nil, err
		}
		f, err := openTrusted(dir, true)
		if err != nil {
			return nil, err
		}
		var st syscall.Stat_t
		err = syscall.Fstat(int(f.Fd()), &st)
		f.Close()
		if err != nil || device(st.Dev) != sc.Mount.Device {
			return nil, errors.New("Pod log target is not on trusted scratch")
		}
	}
	if err = redirectPodLogEntry(parent, filepath.Base(path), target); err != nil {
		return nil, err
	}
	return &podLogs{Path: path, Target: target}, nil
}

// The proc FD path anchors entry operations to the held directory capability.
// A single validated basename cannot escape it; the caller retains the FD.
func redirectPodLogEntry(parent *os.File, name, target string) error {
	if name == "." || name == ".." || filepath.Base(name) != name || name == "" {
		return errors.New("invalid Pod log entry")
	}
	path := fmt.Sprintf("/proc/self/fd/%d/%s", parent.Fd(), name)
	st, err := os.Lstat(path)
	if err != nil {
		return err
	}
	if st.Mode()&os.ModeSymlink != 0 {
		_, err = verifyPodLogLink(path, target)
		return err
	}
	if !st.IsDir() {
		return errors.New("Pod log path is neither the expected symlink nor an empty directory")
	}
	fd, err := syscall.Openat(int(parent.Fd()), name, syscall.O_RDONLY|syscall.O_DIRECTORY|syscall.O_NOFOLLOW|syscall.O_CLOEXEC, 0)
	if err != nil {
		return err
	}
	child := os.NewFile(uintptr(fd), name)
	var stat syscall.Stat_t
	err = syscall.Fstat(fd, &stat)
	if err != nil || stat.Uid != 0 || stat.Mode&0022 != 0 {
		child.Close()
		return errors.New("untrusted Pod log directory")
	}
	entries, err := child.ReadDir(1)
	child.Close()
	if err != nil && !errors.Is(err, io.EOF) {
		return err
	}
	if len(entries) != 0 {
		return errors.New("Pod log directory is not empty")
	}
	// Empty-only removal also rejects a concurrent kubelet writer.
	if err = syscall.Rmdir(path); err != nil {
		return fmt.Errorf("remove empty Pod log directory: %w", err)
	}
	if err = os.Symlink(target, path); err != nil {
		if os.IsExist(err) {
			_, err = verifyPodLogLink(path, target)
		}
		return err
	}
	if err = parent.Sync(); err != nil {
		return err
	}
	_, err = verifyPodLogLink(path, target)
	return err
}

func verifyPodLogLink(path, target string) (*podLogs, error) {
	actual, err := os.Readlink(path)
	if err != nil || actual != target {
		return nil, errors.New("Pod log symlink has an unexpected target")
	}
	return &podLogs{Path: path, Target: target}, nil
}
