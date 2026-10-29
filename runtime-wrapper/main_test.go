package main

import (
 "strings"
 "testing"
)
func TestShimArguments(t *testing.T) {
 args:=[]string{"--root","/run/runc/k8s.io","--log","/tmp/log","--log-format","json","create","--bundle","/run/tasks/"+strings.Repeat("a",64),"--pid-file","/tmp/pid","--console-socket","/tmp/socket","--preserve-fds","3",strings.Repeat("a",64)}
 got,err:=parseArgs(args);if err!=nil||got.Command!="create"||got.ID!=strings.Repeat("a",64)||got.Bundle!="/run/tasks/"+got.ID{t.Fatalf("%+v %v",got,err)}
 for _,bad:=range [][]string{{"--unsafe","create"},{"create","--bundle"},{"create","a","b"},{"create","--bundle=x","--bundle=y","a"},{"create","--restore","a"}} {if _,err:=parseArgs(bad);err==nil{t.Fatalf("accepted %v",bad)}}
}
func TestMountEscapes(t *testing.T) {
 m,err:=decodeMount(`11 2 252:18 /data/tmp /safe\040path rw,relatime shared:1 - xfs /dev/mapper/scratch rw,nouuid`)
 if err!=nil||m.Target!="/safe path"||m.Root!="/data/tmp"||m.Device!="252:18"{t.Fatalf("%+v %v",m,err)}
 for _,bad:=range []string{`\041`,`\04`,`\`}{if _,err:=unescape(bad);err==nil{t.Fatalf("accepted %q",bad)}}
}
func TestBoundaryAndDevice(t *testing.T) {
 if beneath("/safe/data","/safe/data-other/file")||!beneath("/safe/data","/safe/data/file"){t.Fatal("path boundary")}
 // Linux makedev(252, 1059), including the high minor bits.
 dev:=uint64((252<<8)|(1059&255)|((1059&^255)<<12));if device(dev)!="252:1059"{t.Fatal(device(dev))}
}
