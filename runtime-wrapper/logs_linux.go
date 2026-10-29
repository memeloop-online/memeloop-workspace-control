package main

import (
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"regexp"
	"syscall"
)

type podLogs struct {
	Path   string `json:"path"`
	Target string `json:"target"`
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
	parent, err := openTrusted(c.PodLogRoot, true)
	if err != nil {
		return nil, err
	}
	parent.Close()
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
	st, err := os.Lstat(path)
	if err != nil {
		return nil, err
	}
	if st.Mode()&os.ModeSymlink != 0 {
		return verifyPodLogLink(path, target)
	}
	if !st.IsDir() {
		return nil, errors.New("Pod log path is neither the expected symlink nor an empty directory")
	}
	f, err := openTrusted(path, true)
	if err != nil {
		return nil, err
	}
	entries, err := f.ReadDir(1)
	f.Close()
	if err != nil && !errors.Is(err, io.EOF) {
		return nil, err
	}
	if len(entries) != 0 {
		return nil, errors.New("Pod log directory is not empty")
	}
	// rmdir never removes a nonempty directory, including a concurrent writer.
	if err = syscall.Rmdir(path); err != nil {
		return nil, fmt.Errorf("remove empty Pod log directory: %w", err)
	}
	// The short rmdir/symlink gap is intentional. A subsequent kubelet retry can
	// recreate the empty directory; no recursive delete, overwrite or fallback.
	if err = os.Symlink(target, path); err != nil {
		if os.IsExist(err) {
			return verifyPodLogLink(path, target)
		}
		return nil, err
	}
	if err = syncDirectory(c.PodLogRoot); err != nil {
		return nil, err
	}
	return verifyPodLogLink(path, target)
}

func verifyPodLogLink(path, target string) (*podLogs, error) {
	actual, err := os.Readlink(path)
	if err != nil || actual != target {
		return nil, errors.New("Pod log symlink has an unexpected target")
	}
	return &podLogs{Path: path, Target: target}, nil
}
