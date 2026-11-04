package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"syscall"
)

type managedFile struct {
	Destination string `json:"destination"`
	Original    string `json:"original"`
	Redirected  string `json:"redirected"`
	Name        string `json:"name"`
}

// Only the current CRI-managed files and kubelet's actual hosts override are
// supported. An OCI source never selects an arbitrary host path to copy.
func managedFileSources(c config, s spec) ([]managedFile, error) {
	sandbox := s.Annotations["io.kubernetes.cri.sandbox-id"]
	if !idRE.MatchString(sandbox) {
		return nil, errors.New("expected actual CRI sandbox ID")
	}
	sandboxRoot := filepath.Join(c.CRIRoot, "sandboxes", sandbox)
	hosts := filepath.Join(c.KubeletRoot, "pods", s.Annotations["io.kubernetes.cri.sandbox-uid"], "etc-hosts")
	seen := make(map[string]bool)
	var files []managedFile
	for _, m := range s.Mounts {
		name := ""
		switch m.Destination {
		case "/etc/hostname":
			name = "hostname"
		case "/etc/hosts":
			name = "hosts"
		case "/etc/resolv.conf":
			name = "resolv.conf"
		default:
			continue
		}
		if seen[m.Destination] || m.Type != "bind" {
			return nil, fmt.Errorf("unsupported managed file mount %s", m.Destination)
		}
		seen[m.Destination] = true
		if m.Destination == "/etc/hosts" && m.Source == hosts {
			name = "kubelet-hosts"
		} else if m.Source != filepath.Join(sandboxRoot, name) {
			return nil, fmt.Errorf("untrusted managed file source for %s", m.Destination)
		}
		f, err := openTrusted(m.Source, false)
		if err != nil {
			return nil, err
		}
		st, err := f.Stat()
		f.Close()
		if err != nil || !st.Mode().IsRegular() {
			return nil, errors.New("managed file source must be a protected regular file")
		}
		files = append(files, managedFile{Destination: m.Destination, Original: m.Source, Name: name})
	}
	return files, nil
}

// The caller holds the Pod lock and has revalidated scratch. Files are shared by
// source within one sandbox, preserving cross-container writes. A new sandbox
// seeds fresh files, including hosts after a Pod-IP change. Individual container
// deletion must not remove files still used by its peers. These small private
// files live until the scratch volume is reclaimed; no orphan GC is implied.
func redirectManagedFiles(c config, bundle string, sc scratch, s spec, files []managedFile) ([]managedFile, error) {
	if len(files) == 0 {
		return files, nil
	}
	dir := filepath.Join(sc.Path, "runtime-private", "managed-files")
	for _, p := range []string{dir, filepath.Join(dir, s.Annotations["io.kubernetes.cri.sandbox-id"])} {
		if err := os.Mkdir(p, 0700); err != nil && !os.IsExist(err) {
			return nil, err
		}
		f, err := openTrusted(p, true)
		if err != nil {
			return nil, err
		}
		f.Close()
	}
	dir = filepath.Join(dir, s.Annotations["io.kubernetes.cri.sandbox-id"])
	for i := range files {
		files[i].Redirected = filepath.Join(dir, files[i].Name)
		if err := seedManagedFile(files[i], sc); err != nil {
			return nil, err
		}
	}
	if err := rewriteMountSources(filepath.Join(bundle, "config.json"), files); err != nil {
		return nil, err
	}
	return files, nil
}

func seedManagedFile(m managedFile, sc scratch) error {
	if f, err := openPath(m.Redirected, false); err == nil {
		// The container may change this file's owner/mode through its bind mount.
		// Its protected parent and same-scratch inode, rather than mutable leaf
		// permissions, establish ownership of an existing shared file.
		defer f.Close()
		var st syscall.Stat_t
		if err = syscall.Fstat(int(f.Fd()), &st); err != nil {
			return err
		}
		if st.Mode&syscall.S_IFMT != syscall.S_IFREG || device(st.Dev) != sc.Mount.Device {
			return errors.New("invalid existing managed scratch file")
		}
		return nil
	} else if !errors.Is(err, os.ErrNotExist) {
		return err
	}
	source, err := openTrusted(m.Original, false)
	if err != nil {
		return err
	}
	defer source.Close()
	tmp, err := os.CreateTemp(filepath.Dir(m.Redirected), ".seed-")
	if err != nil {
		return err
	}
	defer os.Remove(tmp.Name())
	defer tmp.Close()
	if _, err = io.Copy(tmp, source); err != nil {
		return err
	}
	if err = copyMetadata(m.Original, tmp.Name()); err != nil {
		return err
	}
	if err = tmp.Sync(); err != nil {
		return err
	}
	if err = os.Rename(tmp.Name(), m.Redirected); err != nil {
		return err
	}
	return syncDirectory(filepath.Dir(m.Redirected))
}

// Patch raw JSON fields instead of serializing the intentionally partial spec
// struct: capabilities, seccomp, namespaces, hooks and unknown OCI fields must
// survive unchanged. Only the selected mount.source values are replaced.
func patchMountSources(b []byte, files []managedFile) ([]byte, error) {
	var document map[string]json.RawMessage
	if err := json.Unmarshal(b, &document); err != nil {
		return nil, err
	}
	var mounts []map[string]json.RawMessage
	if err := json.Unmarshal(document["mounts"], &mounts); err != nil {
		return nil, err
	}
	for _, f := range files {
		found := 0
		for _, m := range mounts {
			var destination, source string
			if err := json.Unmarshal(m["destination"], &destination); err != nil {
				return nil, err
			}
			if destination != f.Destination {
				continue
			}
			if err := json.Unmarshal(m["source"], &source); err != nil || source != f.Original {
				return nil, errors.New("managed source changed before OCI rewrite")
			}
			found++
			m["source"], _ = json.Marshal(f.Redirected)
		}
		if found != 1 {
			return nil, errors.New("managed mount changed before OCI rewrite")
		}
	}
	var err error
	document["mounts"], err = json.Marshal(mounts)
	if err != nil {
		return nil, err
	}
	return json.Marshal(document)
}

func rewriteMountSources(path string, files []managedFile) error {
	f, err := openTrusted(path, false)
	if err != nil {
		return err
	}
	b, err := io.ReadAll(f)
	f.Close()
	if err != nil {
		return err
	}
	b, err = patchMountSources(b, files)
	if err != nil {
		return err
	}
	tmp, err := os.CreateTemp(filepath.Dir(path), ".config-")
	if err != nil {
		return err
	}
	defer os.Remove(tmp.Name())
	defer tmp.Close()
	if _, err = tmp.Write(b); err != nil {
		return err
	}
	if err = copyMetadata(path, tmp.Name()); err != nil {
		return err
	}
	if err = tmp.Sync(); err != nil {
		return err
	}
	if err = os.Rename(tmp.Name(), path); err != nil {
		return err
	}
	return syncDirectory(filepath.Dir(path))
}

func syncDirectory(path string) error {
	f, err := os.Open(path)
	if err != nil {
		return err
	}
	defer f.Close()
	return f.Sync()
}
