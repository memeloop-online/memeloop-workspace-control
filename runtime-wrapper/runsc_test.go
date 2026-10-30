package main

import (
	"strings"
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
		if err := validateRunscSpec(spec{Annotations: map[string]string{prefix + "override": "x"}}); err == nil {
			t.Fatal("accepted override", prefix)
		}
	}
}
