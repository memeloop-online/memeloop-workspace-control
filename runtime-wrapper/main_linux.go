// MWC native runtime prototype. This binary is deliberately not a production runtime.
package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"regexp"
	"runtime"
	"strconv"
	"strings"
	"syscall"
)

const configPath = "/etc/mwc-runtime-wrapper.json"

var uuidRE = regexp.MustCompile(`^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$`)
var idRE = regexp.MustCompile(`^[0-9a-f]{64}$`)
var lvmRE = regexp.MustCompile(`^[A-Za-z0-9]{32}$`)

type config struct {
	RealRuntime    string `json:"real_runtime"`
	KubeletRoot    string `json:"kubelet_root"`
	BundleRoot     string `json:"bundle_root"`
	SnapshotRoot   string `json:"snapshot_root"`
	StateRoot      string `json:"state_root"`
	MountNamespace string `json:"mount_namespace"`
	ScratchVG      string `json:"scratch_vg_uuid"`
	ScratchPool    string `json:"scratch_pool_dm_uuid"`
	DMSetup        string `json:"dmsetup"`
}
type ociMount struct {
	Destination string   `json:"destination"`
	Type        string   `json:"type"`
	Source      string   `json:"source"`
	Options     []string `json:"options"`
}
type spec struct {
	Root struct {
		Path     string `json:"path"`
		Readonly bool   `json:"readonly"`
	} `json:"root"`
	Annotations map[string]string `json:"annotations"`
	Mounts      []ociMount        `json:"mounts"`
	Linux       struct {
		UIDMappings []json.RawMessage `json:"uidMappings"`
		GIDMappings []json.RawMessage `json:"gidMappings"`
	} `json:"linux"`
}
type mount struct {
	ID       int      `json:"id"`
	Device   string   `json:"device"`
	Root     string   `json:"root"`
	Target   string   `json:"target"`
	Flags    string   `json:"flags"`
	Optional []string `json:"optional"`
	FS       string   `json:"fs"`
	Source   string   `json:"source"`
	Options  string   `json:"options"`
}
type scratch struct {
	Path   string `json:"path"`
	Volume string `json:"volume"`
	Handle string `json:"handle"`
	DMUUID string `json:"dm_uuid"`
	Mount  mount  `json:"mount"`
	Inode  uint64 `json:"inode"`
}
type record struct {
	ID             string     `json:"id"`
	PodUID         string     `json:"pod_uid"`
	Bundle         string     `json:"bundle"`
	Rootfs         string     `json:"rootfs"`
	Scratch        scratch    `json:"scratch"`
	Original       mount      `json:"original"`
	Upper          string     `json:"upper"`
	Work           string     `json:"work"`
	Lower          string     `json:"lower"`
	Data           string     `json:"mount_data"`
	Phase          string     `json:"phase"`
	MountInventory []ociMount `json:"oci_mount_inventory"`
}
type invocation struct {
	Command string
	ID      string
	Bundle  string
}

func fail(err error) { fmt.Fprintln(os.Stderr, "mwc runtime prototype:", err); os.Exit(125) }
func main() {
	c, err := loadConfig()
	if err != nil {
		fail(err)
	}
	in, err := parseArgs(os.Args[1:])
	if err != nil {
		fail(err)
	}
	if in.Command == "restore" || in.Command == "checkpoint" || in.Command == "run" {
		fail(fmt.Errorf("unsupported task creation/checkpoint command %s", in.Command))
	}
	if in.Command != "create" && in.Command != "delete" {
		if err := syscall.Exec(c.RealRuntime, append([]string{c.RealRuntime}, os.Args[1:]...), os.Environ()); err != nil {
			fail(err)
		}
		return
	}
	if !idRE.MatchString(in.ID) {
		fail(errors.New("expected CRI 64-hex container ID"))
	}
	lock, err := lockFile(filepath.Join(c.StateRoot, in.ID+".lock"))
	if err != nil {
		fail(err)
	}
	defer lock.Close()
	if in.Command == "delete" {
		code, err := deleteContainer(c, in)
		if err != nil {
			fail(err)
		}
		os.Exit(code)
	}
	if err := sameNamespace(c); err != nil {
		fail(err)
	}
	rec, err := prepare(c, in)
	if err != nil {
		fail(err)
	}
	if err := switchRoot(c, &rec); err != nil {
		fail(err)
	}
	code, err := runReal(c.RealRuntime, os.Args[1:])
	if err != nil {
		fail(err)
	}
	if code != 0 {
		if err := noTask(c.RealRuntime, os.Args[1:], in.ID); err != nil {
			fail(err)
		}
		// The shim still owns its normal failure cleanup. Remove only our unreferenced
		// mount, never restore the unbounded original and then launch the process.
		if err := cleanup(c, &rec); err != nil {
			fmt.Fprintln(os.Stderr, "mwc runtime prototype failed-create cleanup:", err)
		}
	} else {
		rec.Phase = "created"
		if err := save(c, rec); err != nil {
			fail(err)
		}
	}
	os.Exit(code)
}

// Only runc 1.4.2's used global options are parsed. Arguments are forwarded byte
// for byte, including the caller's --root/log/console socket/process FDs.
func parseArgs(args []string) (invocation, error) {
	var in invocation
	globalValue := map[string]bool{"--root": true, "--log": true, "--log-format": true, "--rootless": true, "--criu": true}
	globalBool := map[string]bool{"--debug": true, "--systemd-cgroup": true, "--help": true, "--version": true, "-v": true, "-h": true}
	i := 0
	for i < len(args) && strings.HasPrefix(args[i], "-") {
		key, _, eq := strings.Cut(args[i], "=")
		if globalValue[key] {
			if !eq {
				i++
				if i >= len(args) {
					return in, errors.New("missing global option value")
				}
			}
		} else if !globalBool[key] {
			return in, fmt.Errorf("unsupported global option %s", key)
		}
		i++
	}
	if i == len(args) {
		in.Command = "version"
		return in, nil
	}
	in.Command = args[i]
	i++
	if in.Command != "create" && in.Command != "delete" {
		return in, nil
	}
	values := map[string]bool{"--bundle": true, "-b": true, "--pid-file": true, "--console-socket": true, "--preserve-fds": true}
	booleans := map[string]bool{"--detach": true, "--no-pivot": true, "--no-new-keyring": true, "--force": true, "-f": true}
	for ; i < len(args); i++ {
		a := args[i]
		if !strings.HasPrefix(a, "-") {
			if in.ID != "" {
				return in, errors.New("multiple container IDs")
			}
			in.ID = a
			continue
		}
		key, value, eq := strings.Cut(a, "=")
		if values[key] {
			if !eq {
				i++
				if i >= len(args) {
					return in, errors.New("missing command option value")
				}
				value = args[i]
			}
			if key == "--bundle" || key == "-b" {
				if in.Bundle != "" {
					return in, errors.New("duplicate bundle")
				}
				in.Bundle = value
			}
		} else if !booleans[key] {
			return in, fmt.Errorf("unsupported %s option %s", in.Command, key)
		}
	}
	return in, nil
}
func loadConfig() (config, error) {
	var c config
	f, err := openTrusted(configPath, false)
	if err != nil {
		return c, err
	}
	defer f.Close()
	d := json.NewDecoder(f)
	d.DisallowUnknownFields()
	if err = d.Decode(&c); err != nil {
		return c, err
	}
	for _, p := range []string{c.RealRuntime, c.KubeletRoot, c.BundleRoot, c.SnapshotRoot, c.StateRoot, c.DMSetup} {
		if !filepath.IsAbs(p) || filepath.Clean(p) != p {
			return c, fmt.Errorf("noncanonical configured path %q", p)
		}
	}
	if !lvmRE.MatchString(c.ScratchVG) || !strings.HasPrefix(c.ScratchPool, "LVM-"+c.ScratchVG) || !strings.HasSuffix(c.ScratchPool, "-tpool") {
		return c, errors.New("expected full trusted scratch VG and thin-pool identity")
	}
	for _, p := range []string{c.RealRuntime, c.DMSetup, c.KubeletRoot, c.BundleRoot, c.SnapshotRoot, c.StateRoot} {
		f, err := openTrusted(p, p != c.RealRuntime && p != c.DMSetup)
		if err != nil {
			return c, err
		}
		f.Close()
	}
	return c, nil
}

// Traverse from an already trusted root without following a symlink at any
// component. CSI mount crossings are allowed; final mount identity is checked.
func openPath(path string, dir bool) (*os.File, error) {
	if !filepath.IsAbs(path) || filepath.Clean(path) != path {
		return nil, fmt.Errorf("noncanonical path %q", path)
	}
	fd, err := syscall.Open("/", syscall.O_RDONLY|syscall.O_DIRECTORY|syscall.O_CLOEXEC, 0)
	if err != nil {
		return nil, err
	}
	parts := strings.Split(strings.TrimPrefix(path, "/"), "/")
	if path == "/" {
		parts = nil
	}
	for i, p := range parts {
		flags := syscall.O_RDONLY | syscall.O_NOFOLLOW | syscall.O_CLOEXEC
		if i < len(parts)-1 || dir {
			flags |= syscall.O_DIRECTORY
		}
		next, e := syscall.Openat(fd, p, flags, 0)
		syscall.Close(fd)
		if e != nil {
			return nil, fmt.Errorf("open %s: %w", path, e)
		}
		fd = next
	}
	return os.NewFile(uintptr(fd), path), nil
}
func openTrusted(path string, dir bool) (*os.File, error) {
	f, err := openPath(path, dir)
	if err != nil {
		return nil, err
	}
	// A protected leaf below a caller-writable ancestor is not a trusted path.
	for parent := filepath.Dir(path); ; parent = filepath.Dir(parent) {
		pf, e := openPath(parent, true)
		if e != nil {
			f.Close()
			return nil, e
		}
		var ps syscall.Stat_t
		e = syscall.Fstat(int(pf.Fd()), &ps)
		pf.Close()
		if e != nil || ps.Uid != 0 || ps.Mode&0022 != 0 {
			f.Close()
			return nil, fmt.Errorf("untrusted ancestor %s", parent)
		}
		if parent == "/" {
			break
		}
	}
	var st syscall.Stat_t
	if err = syscall.Fstat(int(f.Fd()), &st); err != nil || st.Uid != 0 || st.Mode&0022 != 0 {
		f.Close()
		return nil, fmt.Errorf("path is not root-owned and protected: %s", path)
	}
	return f, nil
}
func lockFile(path string) (*os.File, error) {
	fd, err := syscall.Open(path, syscall.O_RDWR|syscall.O_CREAT|syscall.O_NOFOLLOW|syscall.O_CLOEXEC, 0600)
	if err != nil {
		return nil, err
	}
	f := os.NewFile(uintptr(fd), path)
	var st syscall.Stat_t
	if err = syscall.Fstat(fd, &st); err != nil || st.Uid != 0 || st.Mode&0077 != 0 {
		f.Close()
		return nil, errors.New("untrusted lock")
	}
	if err = syscall.Flock(fd, syscall.LOCK_EX); err != nil {
		f.Close()
		return nil, err
	}
	return f, nil
}
func sameNamespace(c config) error {
	expected, err := os.Readlink(c.MountNamespace)
	if err != nil {
		return err
	}
	for _, p := range []string{"/proc/self/ns/mnt", fmt.Sprintf("/proc/%d/ns/mnt", os.Getppid())} {
		n, e := os.Readlink(p)
		if e != nil || n != expected {
			return errors.New("wrapper, shim, and configured containerd mount namespaces differ")
		}
	}
	return nil
}
func decodeMount(s string) (mount, error) {
	var m mount
	left, right, ok := strings.Cut(s, " - ")
	if !ok {
		return m, errors.New("malformed mountinfo separator")
	}
	a, b := strings.Fields(left), strings.Fields(right)
	if len(a) < 6 || len(b) != 3 {
		return m, errors.New("malformed mountinfo fields")
	}
	var err error
	m.ID, err = strconv.Atoi(a[0])
	if err != nil {
		return m, err
	}
	m.Device = a[2]
	m.Root, err = unescape(a[3])
	if err != nil {
		return m, err
	}
	m.Target, err = unescape(a[4])
	if err != nil {
		return m, err
	}
	m.Flags = a[5]
	m.Optional = a[6:]
	m.FS = b[0]
	m.Source, err = unescape(b[1])
	if err != nil {
		return m, err
	}
	m.Options, err = unescape(b[2])
	return m, err
}
func unescape(s string) (string, error) {
	var b strings.Builder
	for i := 0; i < len(s); i++ {
		if s[i] != '\\' {
			b.WriteByte(s[i])
			continue
		}
		if i+3 >= len(s) {
			return "", errors.New("bad mount escape")
		}
		e := s[i+1 : i+4]
		switch e {
		case "040":
			b.WriteByte(' ')
		case "011":
			b.WriteByte('\t')
		case "012":
			b.WriteByte('\n')
		case "134":
			b.WriteByte('\\')
		default:
			return "", errors.New("unsupported mount escape")
		}
		i += 3
	}
	return b.String(), nil
}
func mounts() ([]mount, error) {
	b, err := os.ReadFile("/proc/self/mountinfo")
	if err != nil {
		return nil, err
	}
	var all []mount
	for _, line := range strings.Split(strings.TrimSpace(string(b)), "\n") {
		m, e := decodeMount(line)
		if e != nil {
			return nil, e
		}
		all = append(all, m)
	}
	return all, nil
}
func atMount(all []mount, path string) (mount, error) {
	var found []mount
	for _, m := range all {
		if m.Target == path {
			found = append(found, m)
		}
	}
	if len(found) != 1 {
		return mount{}, fmt.Errorf("expected one mount at %s, got %d", path, len(found))
	}
	return found[0], nil
}
func beneath(root, path string) bool { return strings.HasPrefix(path, root+"/") }
func device(st uint64) string {
	major := ((st >> 8) & 0xfff) | ((st >> 32) & 0xfffff000)
	minor := (st & 0xff) | ((st >> 12) & 0xffffff00)
	return fmt.Sprintf("%d:%d", major, minor)
}
func dmIdentity(c config, dev string) (string, error) {
	if !regexp.MustCompile(`^[0-9]+:[0-9]+$`).MatchString(dev) {
		return "", errors.New("bad device number")
	}
	base := "/sys/dev/block/" + dev
	raw, err := os.ReadFile(base + "/dm/uuid")
	if err != nil {
		return "", err
	}
	id := strings.TrimSpace(string(raw))
	if len(id) != 68 || !strings.HasPrefix(id, "LVM-"+c.ScratchVG) {
		return "", errors.New("not a thin LV in the configured scratch VG")
	}
	slaves, err := os.ReadDir(base + "/slaves")
	if err != nil || len(slaves) != 1 {
		return "", errors.New("unexpected DM slave topology")
	}
	raw, err = os.ReadFile(base + "/slaves/" + slaves[0].Name() + "/dm/uuid")
	if err != nil || strings.TrimSpace(string(raw)) != c.ScratchPool {
		return "", errors.New("scratch thin pool identity mismatch")
	}
	poolDev, err := os.ReadFile(base + "/slaves/" + slaves[0].Name() + "/dev")
	if err != nil {
		return "", err
	}
	name, err := os.ReadFile(base + "/dm/name")
	if err != nil {
		return "", err
	}
	n := strings.TrimSpace(string(name))
	if !regexp.MustCompile(`^[A-Za-z0-9+_.-]+$`).MatchString(n) {
		return "", errors.New("unsupported DM name")
	}
	table, err := exec.Command(c.DMSetup, "table", n).Output()
	if err != nil {
		return "", err
	}
	fields := strings.Fields(string(table))
	if len(fields) != 5 || fields[0] != "0" || fields[2] != "thin" || fields[3] != strings.TrimSpace(string(poolDev)) {
		return "", errors.New("device is not a direct thin target in the expected pool")
	}
	return id, nil
}
func resolve(c config, pod string, all []mount) (scratch, error) {
	var matches []scratch
	base := filepath.Join(c.KubeletRoot, "pods", pod, "volumes", "kubernetes.io~csi")
	f, err := openTrusted(base, true)
	if err != nil {
		return scratch{}, err
	}
	entries, err := f.ReadDir(-1)
	f.Close()
	if err != nil {
		return scratch{}, err
	}
	for _, entry := range entries {
		if !entry.IsDir() || !strings.HasPrefix(entry.Name(), "pvc-") || !uuidRE.MatchString(strings.TrimPrefix(entry.Name(), "pvc-")) {
			return scratch{}, errors.New("unsupported CSI volume entry")
		}
		p := filepath.Join(base, entry.Name(), "mount")
		m, err := atMount(all, p)
		if err != nil {
			return scratch{}, err
		}
		if m.FS != "xfs" || m.Root != "/" || !strings.HasPrefix(m.Flags, "rw") {
			continue
		}
		id, err := dmIdentity(c, m.Device)
		if err != nil {
			continue
		} // Home has another protected VG.
		vf, err := openTrusted(filepath.Join(base, entry.Name(), "vol_data.json"), false)
		if err != nil {
			return scratch{}, err
		}
		var v struct {
			Driver string `json:"driverName"`
			Handle string `json:"volumeHandle"`
			PV     string `json:"specVolID"`
			Mode   string `json:"volumeLifecycleMode"`
		}
		err = json.NewDecoder(vf).Decode(&v)
		vf.Close()
		if err != nil {
			return scratch{}, err
		}
		if v.Driver != "topolvm.io" || v.PV != entry.Name() || !uuidRE.MatchString(v.Handle) || v.Mode != "Persistent" {
			return scratch{}, errors.New("CSI volume metadata does not match the supported TopoLVM contract")
		}
		// TopoLVM logical-volume identity is also the LV name, not a caller-selected host path.
		name, err := os.ReadFile("/sys/dev/block/" + m.Device + "/dm/name")
		if err != nil || !strings.HasSuffix(strings.TrimSpace(string(name)), "-"+strings.ReplaceAll(v.Handle, "-", "--")) {
			return scratch{}, errors.New("CSI handle and DM LV name differ")
		}
		sf, err := openTrusted(p, true)
		if err != nil {
			return scratch{}, err
		}
		var st syscall.Stat_t
		err = syscall.Fstat(int(sf.Fd()), &st)
		sf.Close()
		if err != nil || device(st.Dev) != m.Device {
			return scratch{}, errors.New("scratch FD identity mismatch")
		}
		matches = append(matches, scratch{Path: p, Volume: entry.Name(), Handle: v.Handle, DMUUID: id, Mount: m, Inode: st.Ino})
	}
	if len(matches) != 1 {
		return scratch{}, fmt.Errorf("expected exactly one trusted scratch publish, got %d", len(matches))
	}
	return matches[0], nil
}
func revalidate(c config, s scratch) error {
	all, err := mounts()
	if err != nil {
		return err
	}
	m, err := atMount(all, s.Path)
	if err != nil {
		return err
	}
	if m.ID != s.Mount.ID || m.Device != s.Mount.Device || m.Root != "/" {
		return errors.New("scratch publish changed")
	}
	f, err := openTrusted(s.Path, true)
	if err != nil {
		return err
	}
	defer f.Close()
	var st syscall.Stat_t
	if err = syscall.Fstat(int(f.Fd()), &st); err != nil || st.Ino != s.Inode || device(st.Dev) != s.Mount.Device {
		return errors.New("scratch inode/device changed")
	}
	id, err := dmIdentity(c, m.Device)
	if err != nil || id != s.DMUUID {
		return errors.New("scratch DM identity changed")
	}
	return nil
}

func overlay(c config, m mount) (lower, upper, work string, flags uintptr, data []string, err error) {
	if m.FS != "overlay" || m.Root != "/" || len(m.Optional) != 0 {
		err = errors.New("requires one private active overlay mount, no propagation/idmap")
		return
	}
	for _, o := range strings.Split(m.Flags, ",") {
		switch o {
		case "rw":
		case "relatime":
			flags |= syscall.MS_RELATIME
		case "noatime":
			flags |= syscall.MS_NOATIME
		case "nodiratime":
			flags |= syscall.MS_NODIRATIME
		case "nosuid":
			flags |= syscall.MS_NOSUID
		case "nodev":
			flags |= syscall.MS_NODEV
		case "noexec":
			flags |= syscall.MS_NOEXEC
		case "strictatime":
			flags |= syscall.MS_STRICTATIME
		default:
			err = fmt.Errorf("unsupported overlay mount flag %s", o)
			return
		}
	}
	for _, o := range strings.Split(m.Options, ",") {
		key, value, _ := strings.Cut(o, "=")
		switch key {
		case "rw":
		case "lowerdir":
			lower = value
		case "upperdir":
			upper = value
		case "workdir":
			work = value
		case "index", "metacopy", "redirect_dir", "xino", "userxattr", "volatile", "uuid":
			data = append(data, o)
		default:
			err = fmt.Errorf("unsupported overlay super option %s", key)
			return
		}
	}
	if lower == "" || upper == "" || work == "" {
		err = errors.New("read-only/bind/rootless snapshots are unsupported")
		return
	}
	for _, p := range append(strings.Split(lower, ":"), upper, work) {
		if !beneath(c.SnapshotRoot, p) || strings.ContainsAny(p, "\\,\n\t ") {
			err = errors.New("unsupported lower/upper/work path")
			return
		}
		f, e := openTrusted(p, true)
		if e != nil {
			err = e
			return
		}
		f.Close()
	}
	return
}
func prepare(c config, in invocation) (record, error) {
	var r record
	if in.Bundle == "" {
		return r, errors.New("explicit shim bundle required")
	}
	if in.Bundle != filepath.Join(c.BundleRoot, in.ID) {
		return r, errors.New("bundle does not match trusted shim container ID")
	}
	bf, err := openTrusted(in.Bundle, true)
	if err != nil {
		return r, err
	}
	bf.Close()
	if _, err = os.Lstat(filepath.Join(c.StateRoot, in.ID+".json")); !os.IsNotExist(err) {
		return r, errors.New("same container ID already has runtime state; recovery is not implemented")
	}
	cf, err := openTrusted(filepath.Join(in.Bundle, "config.json"), false)
	if err != nil {
		return r, err
	}
	var s spec
	err = json.NewDecoder(cf).Decode(&s)
	cf.Close()
	if err != nil {
		return r, err
	}
	pod := s.Annotations["io.kubernetes.cri.sandbox-uid"]
	if !uuidRE.MatchString(pod) || s.Annotations["io.kubernetes.cri.sandbox-namespace"] != "mwc-workspaces" {
		return r, errors.New("unsupported CRI workspace identity")
	}
	kind := s.Annotations["io.kubernetes.cri.container-type"]
	if kind != "sandbox" && kind != "container" {
		return r, errors.New("missing CRI container type")
	}
	if s.Root.Path != "rootfs" || s.Root.Readonly || len(s.Linux.UIDMappings) > 0 || len(s.Linux.GIDMappings) > 0 {
		return r, errors.New("requires writable native non-idmapped rootfs")
	}
	all, err := mounts()
	if err != nil {
		return r, err
	}
	rootfs := filepath.Join(in.Bundle, "rootfs")
	original, err := atMount(all, rootfs)
	if err != nil {
		return r, err
	}
	for _, m := range all {
		if beneath(rootfs, m.Target) {
			return r, errors.New("rootfs already has child mounts")
		}
	}
	lower, oldUpper, _, _, _, err := overlay(c, original)
	if err != nil {
		return r, err
	}
	entries, err := os.ReadDir(oldUpper)
	if err != nil || len(entries) != 0 {
		return r, errors.New("original snapshot upper is not empty")
	}
	sc, err := resolve(c, pod, all)
	if err != nil {
		return r, err
	}
	// Every volume source on scratch must expose only data, including readonly mounts.
	for _, m := range s.Mounts {
		if !filepath.IsAbs(m.Source) {
			continue
		}
		f, e := openPath(m.Source, false)
		if e != nil {
			return r, e
		}
		var st syscall.Stat_t
		e = syscall.Fstat(int(f.Fd()), &st)
		f.Close()
		if e != nil {
			return r, e
		}
		if device(st.Dev) == sc.Mount.Device {
			var sourceMount mount
			for _, candidate := range all {
				if (candidate.Target == m.Source || beneath(candidate.Target, m.Source)) && len(candidate.Target) > len(sourceMount.Target) {
					sourceMount = candidate
				}
			}
			relative, e := filepath.Rel(sourceMount.Target, m.Source)
			if e != nil {
				return r, e
			}
			fsPath := filepath.Clean(filepath.Join(sourceMount.Root, relative))
			if sourceMount.Device != sc.Mount.Device || (fsPath != "/data" && !beneath("/data", fsPath)) {
				return r, errors.New("scratch OCI mount exposes volume root/private area")
			}
		}
	}
	podLock, err := lockFile(filepath.Join(c.StateRoot, pod+".lock"))
	if err != nil {
		return r, err
	}
	defer podLock.Close()
	if err = revalidate(c, sc); err != nil {
		return r, err
	}
	private := filepath.Join(sc.Path, "runtime-private")
	if err = os.Mkdir(private, 0700); err != nil && !os.IsExist(err) {
		return r, err
	}
	pf, err := openTrusted(private, true)
	if err != nil {
		return r, err
	}
	var ps syscall.Stat_t
	err = syscall.Fstat(int(pf.Fd()), &ps)
	pf.Close()
	if err != nil || ps.Mode&0777 != 0700 {
		return r, errors.New("untrusted runtime-private permissions")
	}
	containerDir := filepath.Join(private, in.ID)
	if err = os.Mkdir(containerDir, 0700); err != nil {
		return r, fmt.Errorf("container directory must be new: %w", err)
	}
	r = record{ID: in.ID, PodUID: pod, Bundle: in.Bundle, Rootfs: rootfs, Scratch: sc, Original: original, Upper: filepath.Join(containerDir, "upper"), Work: filepath.Join(containerDir, "work"), Lower: lower, Phase: "prepared", MountInventory: s.Mounts}
	if err = os.Mkdir(r.Upper, 0755); err != nil {
		return r, err
	}
	if err = os.Mkdir(r.Work, 0700); err != nil {
		return r, err
	}
	if err = copyMetadata(oldUpper, r.Upper); err != nil {
		return r, err
	}
	if err = save(c, r); err != nil {
		return r, err
	}
	return r, nil
}
func copyMetadata(from, to string) error {
	var st syscall.Stat_t
	if err := syscall.Stat(from, &st); err != nil {
		return err
	}
	if err := os.Chown(to, int(st.Uid), int(st.Gid)); err != nil {
		return err
	}
	mode := os.FileMode(st.Mode & 0777)
	if st.Mode&syscall.S_ISUID != 0 {
		mode |= os.ModeSetuid
	}
	if st.Mode&syscall.S_ISGID != 0 {
		mode |= os.ModeSetgid
	}
	if st.Mode&syscall.S_ISVTX != 0 {
		mode |= os.ModeSticky
	}
	if err := os.Chmod(to, mode); err != nil {
		return err
	}
	n, err := syscall.Listxattr(from, nil)
	if err != nil {
		return err
	}
	if n == 0 {
		return nil
	}
	names := make([]byte, n)
	n, err = syscall.Listxattr(from, names)
	if err != nil {
		return err
	}
	for _, name := range strings.Split(strings.TrimRight(string(names[:n]), "\x00"), "\x00") {
		size, e := syscall.Getxattr(from, name, nil)
		if e != nil {
			return e
		}
		value := make([]byte, size)
		size, e = syscall.Getxattr(from, name, value)
		if e != nil {
			return e
		}
		if e = syscall.Setxattr(to, name, value[:size], 0); e != nil {
			return e
		}
	}
	return nil
}
func save(c config, r record) error {
	b, err := json.MarshalIndent(r, "", "  ")
	if err != nil {
		return err
	}
	p := filepath.Join(c.StateRoot, r.ID+".json")
	fd, err := syscall.Open(p+".new", syscall.O_WRONLY|syscall.O_CREAT|syscall.O_TRUNC|syscall.O_NOFOLLOW|syscall.O_CLOEXEC, 0600)
	if err != nil {
		return err
	}
	f := os.NewFile(uintptr(fd), p+".new")
	_, err = f.Write(b)
	if err == nil {
		err = f.Sync()
	}
	f.Close()
	if err != nil {
		return err
	}
	if err = os.Rename(p+".new", p); err != nil {
		return err
	}
	d, err := os.Open(c.StateRoot)
	if err != nil {
		return err
	}
	defer d.Close()
	return d.Sync()
}
func switchRoot(c config, r *record) error {
	if err := revalidate(c, r.Scratch); err != nil {
		return err
	}
	all, err := mounts()
	if err != nil {
		return err
	}
	current, err := atMount(all, r.Rootfs)
	if err != nil || current.ID != r.Original.ID {
		return errors.New("original overlay changed")
	}
	_, _, _, flags, data, err := overlay(c, r.Original)
	if err != nil {
		return err
	}
	data = append([]string{"lowerdir=" + r.Lower, "upperdir=" + r.Upper, "workdir=" + r.Work}, data...)
	r.Data = strings.Join(data, ",")
	if err = save(c, *r); err != nil {
		return err
	}
	if err = syscall.Unmount(r.Rootfs, 0); err != nil {
		return err
	}
	r.Phase = "original-unmounted"
	if err = save(c, *r); err != nil {
		return err
	}
	if err = syscall.Mount("overlay", r.Rootfs, "overlay", flags, r.Data); err != nil {
		// Restore only to permit normal shim cleanup. This invocation always fails;
		// real runc is never called with the original, unbounded writable layer.
		_, _, _, oldFlags, _, _ := overlay(c, r.Original)
		restore := syscall.Mount(r.Original.Source, r.Rootfs, "overlay", oldFlags, strings.TrimPrefix(r.Original.Options, "rw,"))
		return fmt.Errorf("redirect mount failed: %v; original cleanup mount restore: %v", err, restore)
	}
	r.Phase = "redirected"
	return save(c, *r)
}
func cleanup(c config, r *record) error {
	if err := revalidate(c, r.Scratch); err != nil {
		return err
	}
	all, err := mounts()
	if err != nil {
		return err
	}
	for _, m := range all {
		if m.Target == r.Rootfs {
			if m.FS != "overlay" || !strings.Contains(","+m.Options+",", ",upperdir="+r.Upper+",") {
				return errors.New("refusing to unmount foreign rootfs")
			}
			if err = syscall.Unmount(r.Rootfs, 0); err != nil {
				return err
			}
		}
	}
	all, err = mounts()
	if err != nil {
		return err
	}
	for _, m := range all {
		if strings.Contains(m.Options, "upperdir="+r.Upper+",") || strings.HasSuffix(m.Options, "upperdir="+r.Upper) || m.Target == r.Rootfs || beneath(r.Rootfs, m.Target) {
			return errors.New("remaining overlay reference; preserve upper")
		}
	}
	if err = os.RemoveAll(filepath.Dir(r.Upper)); err != nil {
		return err
	}
	r.Phase = "deleted"
	return save(c, *r)
}
func deleteContainer(c config, in invocation) (int, error) {
	// Unmanaged runtime IDs are never adopted. Normal missing-state runtime cleanup
	// is passed through; it cannot create an unbounded task.
	p := filepath.Join(c.StateRoot, in.ID+".json")
	f, err := openTrusted(p, false)
	if os.IsNotExist(err) {
		return runReal(c.RealRuntime, os.Args[1:])
	}
	if err != nil {
		return 0, err
	}
	var r record
	err = json.NewDecoder(f).Decode(&r)
	f.Close()
	if err != nil {
		return 0, err
	}
	if r.ID != in.ID || r.Bundle != filepath.Join(c.BundleRoot, in.ID) || r.Rootfs != filepath.Join(r.Bundle, "rootfs") || r.Upper != filepath.Join(r.Scratch.Path, "runtime-private", in.ID, "upper") {
		return 0, errors.New("runtime record identity mismatch")
	}
	if err = sameNamespace(c); err != nil {
		return 0, err
	}
	code, err := runReal(c.RealRuntime, os.Args[1:])
	if err != nil || code != 0 {
		return code, err
	}
	if r.Phase == "deleted" {
		return code, nil
	}
	if err = cleanup(c, &r); err != nil {
		return 0, err
	}
	return code, nil
}
func runReal(path string, args []string) (int, error) {
	// Preserve every inherited non-CLOEXEC descriptor at its original number.
	// This covers go-runc's --preserve-fds and console/process IO, not just stdio.
	files := []*os.File{os.Stdin, os.Stdout, os.Stderr}
	entries, err := os.ReadDir("/proc/self/fd")
	if err != nil {
		return 0, err
	}
	for _, e := range entries {
		fd, e := strconv.Atoi(e.Name())
		if e != nil || fd < 3 {
			continue
		}
		flags, _, errno := syscall.Syscall(syscall.SYS_FCNTL, uintptr(fd), syscall.F_GETFD, 0)
		if errno != 0 || flags&syscall.FD_CLOEXEC != 0 {
			continue
		}
		for len(files) <= fd {
			files = append(files, nil)
		}
		files[fd] = os.NewFile(uintptr(fd), "inherited")
	}
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	sigs := make(chan os.Signal, 16)
	signal.Notify(sigs, syscall.SIGTERM, syscall.SIGINT, syscall.SIGHUP, syscall.SIGQUIT)
	defer signal.Stop(sigs)
	child, err := os.StartProcess(path, append([]string{path}, args...), &os.ProcAttr{Env: os.Environ(), Files: files, Sys: &syscall.SysProcAttr{Pdeathsig: syscall.SIGKILL}})
	if err != nil {
		return 0, err
	}
	done := make(chan struct{})
	defer close(done)
	go func() {
		for {
			select {
			case sig := <-sigs:
				child.Signal(sig)
			case <-done:
				return
			}
		}
	}()
	state, err := child.Wait()
	if err != nil {
		return 0, err
	}
	if ws, ok := state.Sys().(syscall.WaitStatus); ok && ws.Signaled() {
		return 128 + int(ws.Signal()), nil
	}
	return state.ExitCode(), nil
}

// A nonzero create status is not by itself permission to remove an active upper.
func noTask(path string, args []string, id string) error {
	var globals []string
	for i := 0; i < len(args); i++ {
		if args[i] == "create" {
			globals = append(globals, args[:i]...)
			break
		}
	}
	out, err := exec.Command(path, append(globals, "state", id)...).CombinedOutput()
	if err == nil {
		return errors.New("failed create left a live runtime state; preserve upper")
	}
	if !strings.Contains(string(out), "container does not exist") {
		return fmt.Errorf("cannot prove failed task is absent: %s", strings.TrimSpace(string(out)))
	}
	return nil
}
