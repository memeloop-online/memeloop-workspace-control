package main

import (
	"errors"
	"fmt"
	"strings"
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

func validateRunscSpec(s spec) error {
	for key := range s.Annotations {
		for _, prefix := range []string{"dev.gvisor.spec.rootfs.", "dev.gvisor.spec.mount.", "dev.gvisor.flag.", "dev.gvisor.bundle."} {
			if strings.HasPrefix(key, prefix) {
				return fmt.Errorf("unsupported runsc override annotation %s", key)
			}
		}
	}
	return nil
}
