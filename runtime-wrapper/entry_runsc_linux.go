//go:build mwc_runsc

package main

const configPath = "/etc/mwc-runsc-wrapper.json"

func parseArgs(args []string) (invocation, error) {
	return parseRunscArgs(args)
}
func validateRuntimeSpec(s spec) error {
	return validateRunscSpec(s)
}
func failedCreate(c config, r *record, args []string) error {
	// A failed create is not proof of task absence. Keep this ID's mount/upper
	// until real runsc delete succeeds; never use runc's absence error string or
	// a list result that skips unreadable state as permission to remove it.
	r.Phase = "create-failed"
	return save(c, *r)
}
