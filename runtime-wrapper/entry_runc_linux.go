//go:build !mwc_runsc

package main

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"strings"
)

const configPath = "/etc/mwc-runtime-wrapper.json"

func parseArgs(args []string) (invocation, error) {
	return parseRuncArgs(args)
}
func validateRuntimeSpec(s spec) error {
	return nil
}
func failedCreate(c config, r *record, args []string) error {
	if err := noRuncTask(c.RealRuntime, args, r.ID); err != nil {
		return err
	}
	// The shim still owns its normal failure cleanup. Never launch an original,
	// unbounded upper after the redirect has failed.
	if err := cleanup(c, r); err != nil {
		fmt.Fprintln(os.Stderr, "mwc runtime prototype failed-create cleanup:", err)
	}
	return nil
}

// A nonzero create status is not by itself permission to remove an active upper.
func noRuncTask(path string, args []string, id string) error {
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
