package main

import (
	"strings"
	"syscall"
	"testing"
)

func TestRunscShimArguments(t *testing.T) {
	id := strings.Repeat("a", 64)
	globals := []string{"--root=/run/runsc/k8s.io", "--log=/bundle/log.json", "--log-format=json", "--panic-log=/logs/panic.log", "--overlay2=none", "--file-access-mounts=shared", "--systemd-cgroup=false", "--file-access=shared", "--platform=systrap"}
	args := append(append([]string{}, globals...), "create", "--bundle", "/tasks/"+id, "--pid-file", "/tasks/pid", "--console-socket", "/console", "--user-log", "/logs/user", id)
	got, err := parseRunscArgs(args)
	if err != nil || got.Command != "create" || got.ID != id || got.Bundle != "/tasks/"+id {
		t.Fatalf("%+v %v", got, err)
	}
	for _, bad := range [][]string{
		{"create", "--bundle", "/tasks", id},
		{"--overlay2=root:memory", "create", id},
		{"--file-access=exclusive", "create", id},
		{"--allow-flag-override=true", "create", id},
		{"--ignore-cgroups=true", "create", id},
		{"--root=/a", "--root=/b", "state", id},
		{"delete", id, id},
		append(append([]string{}, globals...), "create", "--bundle", "/a", "--bundle", "/b", id),
		append(append([]string{}, globals...), "create", "--fs-restore-image-path", "/restore", id),
	} {
		if _, err := parseRunscArgs(bad); err == nil {
			t.Fatalf("accepted %v", bad)
		}
	}
	for _, prefix := range []string{"dev.gvisor.spec.rootfs.", "dev.gvisor.spec.mount.", "dev.gvisor.flag.", "dev.gvisor.bundle."} {
		if err := validateRunscSpec(config{}, spec{Annotations: map[string]string{prefix + "override": "x"}}); err == nil {
			t.Fatal("accepted override", prefix)
		}
	}
}

func TestRunscBuiltinSharedMemory(t *testing.T) {
	id := strings.Repeat("a", 64)
	c := config{CRIStateRoot: "/cri-state"}
	const prefix = "dev.gvisor.spec.mount.gvisorinternaldevshm."
	source := "/cri-state/sandboxes/" + id + "/shm"
	const size = uint64(67108864)
	s := spec{Annotations: map[string]string{"io.kubernetes.cri.container-type": "sandbox", "io.kubernetes.cri.sandbox-id": id, prefix + "source": source, prefix + "type": "tmpfs", prefix + "share": "pod", prefix + "options": "rw,size=67108864"}, Mounts: []ociMount{{Destination: "/dev/shm", Type: "tmpfs", Source: source}}}
	// The filesystem gate obtains size from the real host tmpfs. Exercise the
	// exact shim contract here independently of the CI host's mount privileges.
	if err := validateRunscShmHints(c, s, size); err != nil {
		t.Fatal(err)
	}
	for key, value := range map[string]string{prefix + "share": "container", prefix + "type": "bind", prefix + "source": "/arbitrary", prefix + "unknown": "x", "io.kubernetes.cri.container-type": "container"} {
		before, existed := s.Annotations[key]
		s.Annotations[key] = value
		if err := validateRunscShmHints(c, s, size); err == nil {
			t.Fatal("accepted altered built-in hint", key)
		}
		if existed {
			s.Annotations[key] = before
		} else {
			delete(s.Annotations, key)
		}
	}
	for _, options := range []string{
		"rw", "rw,size=0", "rw,size=67108863", "rw,size=67108865",
		"rw,size=18446744073709551616", "size=67108864,rw",
		"rw,size=67108864,size=67108864", "rw,rw,size=67108864",
		"rw,size=67108864,bind", "rw,size=67108864,ro",
	} {
		s.Annotations[prefix+"options"] = options
		if err := validateRunscShmHints(c, s, size); err == nil {
			t.Fatal("accepted altered shared-memory options", options)
		}
	}
	s.Annotations[prefix+"options"] = "rw,size=67108864"
	for _, field := range []string{"source", "type", "share", "options"} {
		key := prefix + field
		before := s.Annotations[key]
		delete(s.Annotations, key)
		if err := validateRunscShmHints(c, s, size); err == nil {
			t.Fatal("accepted missing shared-memory hint", field)
		}
		s.Annotations[key] = before
	}
	if err := validateRunscShmHints(c, s, 0); err == nil {
		t.Fatal("accepted unbounded shared-memory size")
	}
	if err := validateRunscShmHints(c, s, size+1); err == nil {
		t.Fatal("accepted hint size differing from actual host tmpfs")
	}
	for _, mounts := range [][]ociMount{
		nil,
		{s.Mounts[0], s.Mounts[0]},
		{{Destination: "/dev/shm", Type: "bind", Source: source}},
		{{Destination: "/dev/shm", Type: "tmpfs", Source: "/arbitrary"}},
	} {
		altered := s
		altered.Mounts = mounts
		if err := validateRunscShmHints(c, altered, size); err == nil {
			t.Fatal("accepted conflicting shared-memory mounts", mounts)
		}
	}
	// Child hints must fail before any attempt to inspect the sandbox's path.
	s.Annotations["io.kubernetes.cri.container-type"] = "container"
	if err := validateRunscSpec(c, s); err == nil {
		t.Fatal("accepted shared-memory hints on child")
	}
}

func TestRunscSharedMemoryTmpfsSize(t *testing.T) {
	const tmpfsMagic = 0x01021994
	for _, fs := range []syscall.Statfs_t{
		{Type: tmpfsMagic, Blocks: 16384, Frsize: 4096, Bsize: 4096},
		{Type: tmpfsMagic, Blocks: 16384, Bsize: 4096},
	} {
		if size, err := runscShmBytes(fs); err != nil || size != 67108864 {
			t.Fatalf("host tmpfs size: %d, %v", size, err)
		}
	}
	for _, fs := range []syscall.Statfs_t{
		{Type: 0x58465342, Blocks: 16384, Frsize: 4096},
		{Type: tmpfsMagic, Frsize: 4096},
		{Type: tmpfsMagic, Blocks: 16384},
		{Type: tmpfsMagic, Blocks: 16384, Frsize: -1},
		{Type: tmpfsMagic, Blocks: ^uint64(0), Frsize: 4096},
	} {
		if _, err := runscShmBytes(fs); err == nil {
			t.Fatal("accepted non-tmpfs, unbounded, or overflowing shared memory", fs)
		}
	}
}
