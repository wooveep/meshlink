package peer

import (
	"testing"

	"meshlink/server/internal/device"
)

func TestBuildVisiblePeersExcludesSelfAndSorts(t *testing.T) {
	records := []*device.Record{
		{ID: "dev-c", PublicKey: "pk-c", OverlayIP: "100.64.0.3"},
		{ID: "dev-a", PublicKey: "pk-a", OverlayIP: "100.64.0.1", DirectEndpoint: &device.DirectEndpoint{Host: "192.0.2.10", Port: 51820}, AdvertisedRoutes: []string{"10.20.0.0/24"}},
		{ID: "dev-b", PublicKey: "pk-b", OverlayIP: "100.64.0.2"},
	}

	peers := BuildVisiblePeers("dev-b", records)
	if len(peers) != 2 {
		t.Fatalf("expected 2 visible peers, got %d", len(peers))
	}

	if peers[0].GetPeerId() != "dev-a" || peers[1].GetPeerId() != "dev-c" {
		t.Fatalf("expected peers sorted by id, got %s then %s", peers[0].GetPeerId(), peers[1].GetPeerId())
	}
	if peers[0].GetAllowedIps()[0] != "100.64.0.1/32" {
		t.Fatalf("expected allowed ip to include overlay /32, got %v", peers[0].GetAllowedIps())
	}
	if peers[0].GetAllowedIps()[1] != "10.20.0.0/24" {
		t.Fatalf("expected advertised route to propagate, got %v", peers[0].GetAllowedIps())
	}
	if peers[0].GetDirectEndpoint().GetHost() != "192.0.2.10" || peers[0].GetDirectEndpoint().GetPort() != 51820 {
		t.Fatalf("expected direct endpoint to propagate, got %+v", peers[0].GetDirectEndpoint())
	}
}
