package app

import (
	"net"
	"testing"

	"meshlink/server/internal/device"
)

func TestNormalizeAdvertisedRoutesSortsAndDeduplicates(t *testing.T) {
	routes, err := normalizeAdvertisedRoutes([]string{
		"10.30.0.0/24",
		"10.20.0.0/24",
		"10.30.0.0/24",
		" 10.20.0.0/24 ",
	})
	if err != nil {
		t.Fatalf("normalize advertised routes: %v", err)
	}

	expected := []string{"10.20.0.0/24", "10.30.0.0/24"}
	if len(routes) != len(expected) {
		t.Fatalf("expected %d routes, got %v", len(expected), routes)
	}
	for index, value := range expected {
		if routes[index] != value {
			t.Fatalf("expected route %q at index %d, got %v", value, index, routes)
		}
	}
}

func TestNormalizeAdvertisedRoutesRejectsDefaultAndIPv6(t *testing.T) {
	for _, route := range []string{"0.0.0.0/0", "2001:db8::/64"} {
		if _, err := normalizeAdvertisedRoutes([]string{route}); err == nil {
			t.Fatalf("expected route %s to be rejected", route)
		}
	}
}

func TestValidateAdvertisedRoutesRejectsOverlayOverlap(t *testing.T) {
	_, overlayNet, _ := net.ParseCIDR("100.64.0.0/24")
	err := validateAdvertisedRoutes([]string{"100.64.0.0/25"}, overlayNet, nil, "pk-a")
	if err == nil {
		t.Fatal("expected overlay overlap to be rejected")
	}
}

func TestValidateAdvertisedRoutesRejectsOverlapAcrossDevices(t *testing.T) {
	_, overlayNet, _ := net.ParseCIDR("100.64.0.0/24")
	err := validateAdvertisedRoutes(
		[]string{"10.20.0.128/25"},
		overlayNet,
		[]*device.Record{
			{ID: "dev-b", PublicKey: "pk-b", AdvertisedRoutes: []string{"10.20.0.0/24"}},
		},
		"pk-a",
	)
	if err == nil {
		t.Fatal("expected overlapping advertised routes to be rejected")
	}
}
