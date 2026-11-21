package main

import (
	"encoding/json"
	"reflect"
	"strings"
	"testing"
)

func TestShimArguments(t *testing.T) {
	args := []string{"--root", "/run/runc/k8s.io", "--log", "/tmp/log", "--log-format", "json", "create", "--bundle", "/run/tasks/" + strings.Repeat("a", 64), "--pid-file", "/tmp/pid", "--console-socket", "/tmp/socket", "--preserve-fds", "3", strings.Repeat("a", 64)}
	got, err := parseRuncArgs(args)
	if err != nil || got.Command != "create" || got.ID != strings.Repeat("a", 64) || got.Bundle != "/run/tasks/"+got.ID {
		t.Fatalf("%+v %v", got, err)
	}
	for _, bad := range [][]string{{"--unsafe", "create"}, {"create", "--bundle"}, {"create", "a", "b"}, {"create", "--bundle=x", "--bundle=y", "a"}, {"create", "--restore", "a"}} {
		if _, err := parseRuncArgs(bad); err == nil {
			t.Fatalf("accepted %v", bad)
		}
	}
}

func TestOCIPatchPreservesUnknownFields(t *testing.T) {
	before := []byte(`{"ociVersion":"1.3.0","process":{"capabilities":{"bounding":["CAP_CHOWN"]}},"linux":{"seccomp":{"defaultAction":"SCMP_ACT_ERRNO"}},"future":{"integer":9007199254740993},"hooks":{"prestart":[]},"mounts":[{"destination":"/etc/hosts","type":"bind","source":"/trusted/hosts","options":["rbind","rprivate","rw"],"uidMappings":[{"size":1}],"futureMount":true},{"destination":"/home/user","type":"bind","source":"/home-pvc","options":["rw"]}]}`)
	files := []managedFile{{Destination: "/etc/hosts", Original: "/trusted/hosts", Redirected: "/scratch/private/hosts"}}
	after, err := patchMountSources(before, files)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(after), "9007199254740993") {
		t.Fatal("unknown numeric field changed")
	}
	var expected, actual any
	if err = json.Unmarshal([]byte(strings.Replace(string(before), "/trusted/hosts", "/scratch/private/hosts", 1)), &expected); err != nil {
		t.Fatal(err)
	}
	if err = json.Unmarshal(after, &actual); err != nil || !reflect.DeepEqual(expected, actual) {
		t.Fatalf("OCI fields changed: %s, %v", after, err)
	}
	files[0].Original = "/changed"
	if _, err = patchMountSources(before, files); err == nil {
		t.Fatal("changed source accepted")
	}
}
func TestMountEscapes(t *testing.T) {
	m, err := decodeMount(`11 2 252:18 /data/tmp /safe\040path rw,relatime shared:1 - xfs /dev/mapper/scratch rw,nouuid`)
	if err != nil || m.Target != "/safe path" || m.Root != "/data/tmp" || m.Device != "252:18" {
		t.Fatalf("%+v %v", m, err)
	}
	for _, bad := range []string{`\041`, `\04`, `\`} {
		if _, err := unescape(bad); err == nil {
			t.Fatalf("accepted %q", bad)
		}
	}
}
func TestBoundaryAndDevice(t *testing.T) {
	if beneath("/safe/data", "/safe/data-other/file") || !beneath("/safe/data", "/safe/data/file") {
		t.Fatal("path boundary")
	}
	// Linux makedev(252, 1059), including the high minor bits.
	dev := uint64((252 << 8) | (1059 & 255) | ((1059 &^ 255) << 12))
	if device(dev) != "252:1059" {
		t.Fatal(device(dev))
	}
}
