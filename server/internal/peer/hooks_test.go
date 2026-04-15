package peer

import (
	"testing"

	"meshlink/server/internal/device"
	"meshlink/server/pkg/pb"
)

type testHook struct {
	name  string
	apply func(self *device.Record, peer *device.Record, visible *pb.Peer) error
}

func (h testHook) Name() string {
	return h.name
}

func (h testHook) Apply(self *device.Record, peer *device.Record, visible *pb.Peer) error {
	return h.apply(self, peer, visible)
}

func TestStaticRouteAdvertiserHookBuildsAllowedIPs(t *testing.T) {
	visible := &pb.Peer{}
	err := StaticRouteAdvertiserHook{}.Apply(
		&device.Record{ID: "dev-a"},
		&device.Record{
			ID:               "dev-b",
			OverlayIP:        "100.64.0.2",
			AdvertisedRoutes: []string{"10.20.0.0/24", "10.30.0.0/24"},
		},
		visible,
	)
	if err != nil {
		t.Fatalf("apply hook: %v", err)
	}

	expected := []string{"100.64.0.2/32", "10.20.0.0/24", "10.30.0.0/24"}
	if len(visible.GetAllowedIps()) != len(expected) {
		t.Fatalf("expected %d allowed IPs, got %v", len(expected), visible.GetAllowedIps())
	}
	for index, value := range expected {
		if visible.GetAllowedIps()[index] != value {
			t.Fatalf("expected allowed IP %q at index %d, got %v", value, index, visible.GetAllowedIps())
		}
	}
}

func TestBuildVisiblePeersAppliesHooksInOrder(t *testing.T) {
	records := []*device.Record{
		{ID: "dev-a", PublicKey: "pk-a", OverlayIP: "100.64.0.1"},
		{ID: "dev-b", PublicKey: "pk-b", OverlayIP: "100.64.0.2"},
	}

	peers := BuildVisiblePeers("dev-a", records,
		testHook{
			name: "first",
			apply: func(_ *device.Record, _ *device.Record, visible *pb.Peer) error {
				visible.AllowedIps = append(visible.AllowedIps, "first")
				return nil
			},
		},
		testHook{
			name: "second",
			apply: func(_ *device.Record, _ *device.Record, visible *pb.Peer) error {
				visible.AllowedIps = append(visible.AllowedIps, "second")
				return nil
			},
		},
	)
	if len(peers) != 1 {
		t.Fatalf("expected one visible peer, got %d", len(peers))
	}

	expected := []string{"first", "second"}
	for index, value := range expected {
		if peers[0].GetAllowedIps()[index] != value {
			t.Fatalf("expected hook output %q at index %d, got %v", value, index, peers[0].GetAllowedIps())
		}
	}
}
