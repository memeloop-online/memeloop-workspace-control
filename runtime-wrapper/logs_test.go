package main

import (
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"syscall"
	"testing"
	"unsafe"
)

func TestPodLogRootKeepsOrdinaryTrustPolicy(t *testing.T) {
	f, err := openPodLogRoot("/")
	if err != nil {
		t.Fatal(err)
	}
	f.Close()
	if _, err := openPodLogRoot(t.TempDir()); err == nil {
		t.Fatal("accepted caller-writable ancestors")
	}
}

func TestPodLogEntryRejectsForeignLinksAndNeverRemovesContents(t *testing.T) {
	root := t.TempDir()
	parent, err := os.Open(root)
	if err != nil {
		t.Fatal(err)
	}
	defer parent.Close()
	target := filepath.Join(root, "target")
	if err = os.Symlink(target, filepath.Join(root, "expected")); err != nil {
		t.Fatal(err)
	}
	if err = redirectPodLogEntry(parent, "expected", target); err != nil {
		t.Fatal(err)
	}
	if err = os.Symlink("/foreign", filepath.Join(root, "foreign")); err != nil {
		t.Fatal(err)
	}
	if err = redirectPodLogEntry(parent, "foreign", target); err == nil {
		t.Fatal("accepted a foreign symlink")
	}
	if err = os.Mkdir(filepath.Join(root, "nonempty"), 0750); err != nil {
		t.Fatal(err)
	}
	marker := filepath.Join(root, "nonempty", "marker")
	if err = os.WriteFile(marker, []byte("preserved"), 0600); err != nil {
		t.Fatal(err)
	}
	if err = redirectPodLogEntry(parent, "nonempty", target); err == nil {
		t.Fatal("removed a populated or untrusted directory")
	}
	if b, err := os.ReadFile(marker); err != nil || string(b) != "preserved" {
		t.Fatalf("changed original data: %q %v", b, err)
	}
	if actual, err := os.Readlink(filepath.Join(root, "foreign")); err != nil || actual != "/foreign" {
		t.Fatal("changed foreign link")
	}
	for _, name := range []string{"", ".", "..", "../foreign", "/absolute"} {
		if err = redirectPodLogEntry(parent, name, target); err == nil {
			t.Fatalf("accepted entry %q", name)
		}
	}
}

// CI invokes this binary inside a fresh private mount namespace, never on a
// workstation or the host PID1 namespace. Mounts cover only this test's view.
func TestMountedCanonicalLogRoot(t *testing.T) {
	if os.Getenv("GITHUB_ACTIONS") != "true" || os.Getenv("MWC_LOG_BOUNDARY_CI") != "1" {
		t.Skip("disposable mount-boundary CI only")
	}
	if os.Geteuid() != 0 || runtime.GOARCH != "amd64" {
		t.Fatal("boundary CI requires root and linux/amd64")
	}
	ns, err := os.Readlink("/proc/self/ns/mnt")
	if err != nil {
		t.Fatal(err)
	}
	hostNS, err := os.Readlink("/proc/1/ns/mnt")
	if err != nil || ns == hostNS {
		t.Fatal("refusing host mount namespace")
	}
	base, err := os.MkdirTemp("/var/lib", "mwc-log-boundary-")
	if err != nil {
		t.Fatal(err)
	}
	defer os.RemoveAll(base)
	if err = os.Chown(base, 0, 65000); err != nil {
		t.Fatal(err)
	}
	if err = os.Chmod(base, 0775); err != nil {
		t.Fatal(err)
	}
	if err = syscall.Mount(base, "/var/log", "", syscall.MS_BIND, ""); err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := syscall.Unmount("/var/log", 0); err != nil {
			t.Errorf("unmount isolated parent: %v", err)
		}
	}()
	const leaf = "/var/log/pods"
	if err = os.Mkdir(leaf, 0750); err != nil {
		t.Fatal(err)
	}
	marker := filepath.Join(leaf, "marker")
	if err = os.WriteFile(marker, []byte("original"), 0600); err != nil {
		t.Fatal(err)
	}
	original, err := os.Open(leaf)
	if err != nil {
		t.Fatal(err)
	}
	defer original.Close()
	before, err := original.Stat()
	if err != nil {
		t.Fatal(err)
	}
	if f, err := openPodLogRoot(leaf); err == nil {
		f.Close()
		t.Fatal("accepted unmounted leaf under writable parent")
	}
	if err = syscall.Mount(leaf, leaf, "", syscall.MS_BIND, ""); err != nil {
		t.Fatal(err)
	}
	mounted := true
	defer func() {
		if mounted {
			if err := syscall.Unmount(leaf, 0); err != nil {
				t.Errorf("unmount isolated log root: %v", err)
			}
		}
	}()
	f, err := openPodLogRoot(leaf)
	if err != nil {
		t.Fatal(err)
	}
	after, err := f.Stat()
	if err != nil || !os.SameFile(before, after) {
		f.Close()
		t.Fatal("self-bind changed original log directory")
	}
	const entry = "memeloop-workspace-control_fixture_01234567-0123-0123-0123-0123456789ab"
	if err = os.Mkdir(filepath.Join(leaf, entry), 0750); err != nil {
		f.Close()
		t.Fatal(err)
	}
	if err = redirectPodLogEntry(f, entry, "/trusted/scratch/runtime-private/logs/fixture"); err != nil {
		f.Close()
		t.Fatal(err)
	}
	if err = redirectPodLogEntry(f, entry, "/trusted/scratch/runtime-private/logs/fixture"); err != nil {
		f.Close()
		t.Fatal("existing exact link was not idempotent")
	}
	f.Close()
	cmd := exec.Command(os.Args[0], "-test.run=^TestMountedLogRootAttacker$")
	cmd.Env = append(os.Environ(), "MWC_LOG_ATTACKER_NS="+ns)
	cmd.SysProcAttr = &syscall.SysProcAttr{Credential: &syscall.Credential{Uid: 65534, Gid: 65000, Groups: []uint32{65000}}}
	if output, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("unprivileged attack acceptance: %v\n%s", err, output)
	}
	if b, err := os.ReadFile(marker); err != nil || string(b) != "original" {
		t.Fatal("attacker changed original logs")
	}
	if err = os.Chmod(leaf, 0770); err != nil {
		t.Fatal(err)
	}
	if f, err := openPodLogRoot(leaf); err == nil {
		f.Close()
		t.Fatal("accepted writable mounted leaf")
	}
	if err = os.Chmod(leaf, 0750); err != nil {
		t.Fatal(err)
	}
	if err = syscall.Unmount(leaf, 0); err != nil {
		t.Fatal(err)
	}
	mounted = false
	if f, err := openPodLogRoot(leaf); err == nil {
		f.Close()
		t.Fatal("accepted leaf after losing mount boundary")
	}
	still, err := original.Stat()
	if err != nil || !os.SameFile(before, still) {
		t.Fatal("original log FD lost identity")
	}
}

func TestMountedLogRootAttacker(t *testing.T) {
	ns := os.Getenv("MWC_LOG_ATTACKER_NS")
	if ns == "" {
		t.Skip("subprocess of isolated mount-boundary test only")
	}
	actual, err := os.Readlink("/proc/self/ns/mnt")
	if err != nil || actual != ns || os.Geteuid() != 65534 || os.Getegid() != 65000 {
		t.Fatal("attacker must share namespace and lack root identity")
	}
	status, err := os.ReadFile("/proc/self/status")
	if err != nil || !strings.Contains(string(status), "CapEff:\t0000000000000000") {
		t.Fatal("attacker retained effective capabilities")
	}
	const leaf = "/var/log/pods"
	const sibling = "/var/log/attacker"
	if err = os.Mkdir(sibling, 0700); err != nil {
		t.Fatalf("attacker lacks expected parent group-write permission: %v", err)
	}
	if err = os.Rename(leaf, "/var/log/stolen"); !errors.Is(err, syscall.EBUSY) {
		t.Fatalf("rename mountpoint: %v", err)
	}
	if err = syscall.Rmdir(leaf); !errors.Is(err, syscall.EBUSY) {
		t.Fatalf("rmdir mountpoint: %v", err)
	}
	// os.Rename rejects an existing directory in user space before the kernel
	// sees it. Use the syscall to exercise destination mountpoint protection.
	if err = syscall.Rename(sibling, leaf); !errors.Is(err, syscall.EBUSY) {
		t.Fatalf("overwrite mountpoint: %v", err)
	}
	// renameat2(RENAME_EXCHANGE), linux/amd64 as required by the parent gate.
	old, err := syscall.BytePtrFromString(sibling)
	if err != nil {
		t.Fatal(err)
	}
	newPath, err := syscall.BytePtrFromString(leaf)
	if err != nil {
		t.Fatal(err)
	}
	_, _, errno := syscall.Syscall6(316, ^uintptr(99), uintptr(unsafe.Pointer(old)), ^uintptr(99), uintptr(unsafe.Pointer(newPath)), 2, 0)
	if errno != syscall.EBUSY {
		t.Fatalf("exchange mountpoint: %v", errno)
	}
	if err = os.Symlink(sibling, leaf); !errors.Is(err, syscall.EEXIST) {
		t.Fatalf("replace mountpoint with symlink: %v", err)
	}
}
