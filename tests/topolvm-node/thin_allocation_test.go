package lvmd

import (
	"context"
	"os"
	"strings"
	"testing"
	"time"

	"github.com/topolvm/topolvm/internal/lvmd/command"
	"github.com/topolvm/topolvm/pkg/lvmd/proto"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

// Copied into TopoLVM's lvmd package by the hosted CI comparison.
func TestMWCThinAllocationReport(t *testing.T) {
	trace := os.Getenv("MWC_LVM_TRACE")
	command.SetLVMCommandPrefix([]string{os.Getenv("MWC_TRACE_LVM")})
	ctx := context.Background()
	svc, _, vg, pool := setupLVService(ctx, t)

	const gib = 1 << 30
	peer := &proto.CreateLVRequest{Name: "peer", DeviceClass: lvServiceTestThinDC, SizeBytes: 2 * gib}
	if _, err := svc.CreateLV(ctx, peer); err != nil {
		t.Fatal(err)
	}
	// An existing 2 GiB volume leaves only 8 GiB virtual capacity in this pool.
	oversized := &proto.CreateLVRequest{Name: "too-large", DeviceClass: lvServiceTestThinDC, SizeBytes: 9 * gib}
	if _, err := svc.CreateLV(ctx, oversized); status.Code(err) != codes.ResourceExhausted {
		t.Fatalf("oversized allocation should be rejected; got %v", err)
	}
	if vols, err := pool.ListVolumes(ctx); err != nil || len(vols) != 1 || vols["peer"] == nil {
		t.Fatalf("expected only the original volume after refusal: %v, %v", vols, err)
	}

	req := &proto.CreateLVRequest{Name: "measured", DeviceClass: lvServiceTestThinDC, SizeBytes: gib}
	for i := range 13 {
		if err := os.Truncate(trace, 0); err != nil {
			t.Fatal(err)
		}
		start := time.Now()
		res, err := svc.CreateLV(ctx, req)
		elapsed := time.Since(start)
		if err != nil {
			t.Fatal(err)
		}
		raw, err := os.ReadFile(trace)
		if err != nil {
			t.Fatal(err)
		}
		// Verify returned device coordinates against a separate native read.
		got := res.GetVolume()
		lv, err := pool.FindVolume(ctx, req.GetName())
		if err != nil {
			t.Fatal(err)
		}
		if got.GetName() != lv.Name() || got.GetSizeBytes() != int64(lv.Size()) ||
			got.GetDevMajor() != lv.MajorNumber() || got.GetDevMinor() != lv.MinorNumber() {
			t.Fatalf("allocation returned the wrong device: %v", got)
		}
		if _, err := svc.RemoveLV(ctx, &proto.RemoveLVRequest{Name: req.GetName(), DeviceClass: req.GetDeviceClass()}); err != nil {
			t.Fatal(err)
		}
		if i > 0 {
			t.Logf("MWC_ALLOC %d %s", elapsed.Microseconds(), strings.Join(strings.Fields(string(raw)), ","))
		}
	}
	found, err := vg.FindVolume(ctx, peer.GetName())
	if err != nil || found.Size() != uint64(peer.GetSizeBytes()) {
		t.Fatalf("prior allocation changed: %v, %v", found, err)
	}
}
