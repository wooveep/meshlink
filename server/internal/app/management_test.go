package app

import (
	"context"
	"testing"
	"time"

	"google.golang.org/grpc/metadata"

	"meshlink/server/pkg/pb"
)

func TestRegisterDeviceReturnsVisiblePeers(t *testing.T) {
	service := newTestManagementService(t)

	first, err := service.RegisterDevice(context.Background(), &pb.RegisterDeviceRequest{
		Name:      "client-a",
		PublicKey: "pk-a",
		Token:     "meshlink-dev-token",
		Os:        "linux",
		Version:   "0.1.0",
		DirectEndpoint: &pb.DirectEndpoint{
			Host: "192.0.2.10",
			Port: 51820,
		},
		AdvertisedRoutes: []string{"10.20.0.0/24"},
	})
	if err != nil {
		t.Fatalf("register first device: %v", err)
	}
	if len(first.GetPeers()) != 0 {
		t.Fatalf("expected no peers for first device, got %d", len(first.GetPeers()))
	}

	second, err := service.RegisterDevice(context.Background(), &pb.RegisterDeviceRequest{
		Name:      "client-b",
		PublicKey: "pk-b",
		Token:     "meshlink-dev-token",
		Os:        "linux",
		Version:   "0.1.0",
	})
	if err != nil {
		t.Fatalf("register second device: %v", err)
	}
	if len(second.GetPeers()) != 1 {
		t.Fatalf("expected one visible peer, got %d", len(second.GetPeers()))
	}
	if second.GetPeers()[0].GetPeerId() != first.GetDevice().GetId() {
		t.Fatalf("expected peer %s, got %s", first.GetDevice().GetId(), second.GetPeers()[0].GetPeerId())
	}
	if second.GetPeers()[0].GetDirectEndpoint().GetHost() != "192.0.2.10" {
		t.Fatalf("expected visible peer direct endpoint to propagate, got %+v", second.GetPeers()[0].GetDirectEndpoint())
	}
	if len(second.GetPeers()[0].GetAllowedIps()) != 2 || second.GetPeers()[0].GetAllowedIps()[1] != "10.20.0.0/24" {
		t.Fatalf("expected advertised route in allowed ips, got %v", second.GetPeers()[0].GetAllowedIps())
	}
}

func TestSyncConfigPublishesPeerDiscoveryUpdates(t *testing.T) {
	service := newTestManagementService(t)

	first, err := service.RegisterDevice(context.Background(), &pb.RegisterDeviceRequest{
		Name:      "client-a",
		PublicKey: "pk-a",
		Token:     "meshlink-dev-token",
		Os:        "linux",
		Version:   "0.1.0",
	})
	if err != nil {
		t.Fatalf("register first device: %v", err)
	}

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	stream := newSyncConfigStream(ctx)
	errCh := make(chan error, 1)
	go func() {
		errCh <- service.SyncConfig(&pb.SyncConfigRequest{DeviceId: first.GetDevice().GetId()}, stream)
	}()

	full := mustReceiveEvent(t, stream.events)
	if full.GetType() != pb.SyncConfigEventType_SYNC_CONFIG_EVENT_TYPE_FULL {
		t.Fatalf("expected full event, got %s", full.GetType())
	}
	if len(full.GetPeers()) != 0 {
		t.Fatalf("expected no peers in initial full view, got %d", len(full.GetPeers()))
	}

	second, err := service.RegisterDevice(context.Background(), &pb.RegisterDeviceRequest{
		Name:      "client-b",
		PublicKey: "pk-b",
		Token:     "meshlink-dev-token",
		Os:        "linux",
		Version:   "0.1.0",
		DirectEndpoint: &pb.DirectEndpoint{
			Host: "192.0.2.20",
			Port: 51821,
		},
	})
	if err != nil {
		t.Fatalf("register second device: %v", err)
	}

	update := mustReceiveEvent(t, stream.events)
	if update.GetType() != pb.SyncConfigEventType_SYNC_CONFIG_EVENT_TYPE_INCREMENTAL {
		t.Fatalf("expected incremental event, got %s", update.GetType())
	}
	if update.GetRevision() <= full.GetRevision() {
		t.Fatalf("expected revision to advance, got %s then %s", full.GetRevision(), update.GetRevision())
	}
	if len(update.GetPeers()) != 1 {
		t.Fatalf("expected one visible peer after update, got %d", len(update.GetPeers()))
	}
	if update.GetPeers()[0].GetPeerId() != second.GetDevice().GetId() {
		t.Fatalf("expected peer %s, got %s", second.GetDevice().GetId(), update.GetPeers()[0].GetPeerId())
	}
	if update.GetPeers()[0].GetDirectEndpoint().GetHost() != "192.0.2.20" {
		t.Fatalf("expected propagated direct endpoint, got %+v", update.GetPeers()[0].GetDirectEndpoint())
	}

	cancel()
	select {
	case err := <-errCh:
		if err != nil {
			t.Fatalf("sync config returned error: %v", err)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("timed out waiting for sync stream shutdown")
	}
}

func TestRegisterDevicePreservesExistingDirectEndpoint(t *testing.T) {
	service := newTestManagementService(t)

	first, err := service.RegisterDevice(context.Background(), &pb.RegisterDeviceRequest{
		Name:      "client-a",
		PublicKey: "pk-a",
		Token:     "meshlink-dev-token",
		Os:        "linux",
		Version:   "0.1.0",
		DirectEndpoint: &pb.DirectEndpoint{
			Host: "198.51.100.10",
			Port: 51820,
		},
	})
	if err != nil {
		t.Fatalf("register first device: %v", err)
	}

	second, err := service.RegisterDevice(context.Background(), &pb.RegisterDeviceRequest{
		Name:      "client-a-renamed",
		PublicKey: "pk-a",
		Token:     "meshlink-dev-token",
		Os:        "linux",
		Version:   "0.1.1",
	})
	if err != nil {
		t.Fatalf("register second device: %v", err)
	}

	if first.GetDevice().GetId() != second.GetDevice().GetId() {
		t.Fatalf("expected stable device id, got %s then %s", first.GetDevice().GetId(), second.GetDevice().GetId())
	}
	if second.GetDevice().GetDirectEndpoint().GetHost() != "198.51.100.10" || second.GetDevice().GetDirectEndpoint().GetPort() != 51820 {
		t.Fatalf("expected direct endpoint to be preserved, got %+v", second.GetDevice().GetDirectEndpoint())
	}
}

func TestRegisterDeviceRejectsIncompleteDirectEndpoint(t *testing.T) {
	service := newTestManagementService(t)

	_, err := service.RegisterDevice(context.Background(), &pb.RegisterDeviceRequest{
		Name:      "client-a",
		PublicKey: "pk-a",
		Token:     "meshlink-dev-token",
		Os:        "linux",
		Version:   "0.1.0",
		DirectEndpoint: &pb.DirectEndpoint{
			Host: "192.0.2.10",
		},
	})
	if err == nil {
		t.Fatal("expected validation error for incomplete direct endpoint")
	}
}

func TestRegisterDeviceRejectsOverlappingAdvertisedRoutes(t *testing.T) {
	service := newTestManagementService(t)

	_, err := service.RegisterDevice(context.Background(), &pb.RegisterDeviceRequest{
		Name:             "client-a",
		PublicKey:        "pk-a",
		Token:            "meshlink-dev-token",
		Os:               "linux",
		Version:          "0.1.0",
		AdvertisedRoutes: []string{"10.20.0.0/24"},
	})
	if err != nil {
		t.Fatalf("register first device: %v", err)
	}

	_, err = service.RegisterDevice(context.Background(), &pb.RegisterDeviceRequest{
		Name:             "client-b",
		PublicKey:        "pk-b",
		Token:            "meshlink-dev-token",
		Os:               "linux",
		Version:          "0.1.0",
		AdvertisedRoutes: []string{"10.20.0.128/25"},
	})
	if err == nil {
		t.Fatal("expected overlapping advertised route to be rejected")
	}
}

func TestRegisterDeviceClearsAdvertisedRoutesWhenEmptyListProvided(t *testing.T) {
	service := newTestManagementService(t)

	first, err := service.RegisterDevice(context.Background(), &pb.RegisterDeviceRequest{
		Name:             "client-a",
		PublicKey:        "pk-a",
		Token:            "meshlink-dev-token",
		Os:               "linux",
		Version:          "0.1.0",
		AdvertisedRoutes: []string{"10.20.0.0/24"},
	})
	if err != nil {
		t.Fatalf("register first device: %v", err)
	}
	if len(first.GetDevice().GetAdvertisedRoutes()) != 1 {
		t.Fatalf("expected one advertised route, got %v", first.GetDevice().GetAdvertisedRoutes())
	}

	second, err := service.RegisterDevice(context.Background(), &pb.RegisterDeviceRequest{
		Name:             "client-a",
		PublicKey:        "pk-a",
		Token:            "meshlink-dev-token",
		Os:               "linux",
		Version:          "0.1.1",
		AdvertisedRoutes: []string{},
	})
	if err != nil {
		t.Fatalf("register second device: %v", err)
	}

	if len(second.GetDevice().GetAdvertisedRoutes()) != 0 {
		t.Fatalf("expected advertised routes to be cleared, got %v", second.GetDevice().GetAdvertisedRoutes())
	}
}

func newTestManagementService(t *testing.T) *ManagementService {
	t.Helper()

	service, err := NewManagementService(ManagementConfig{
		BootstrapToken: "meshlink-dev-token",
		OverlayCIDR:    "100.64.0.0/24",
		SyncInterval:   time.Hour,
	})
	if err != nil {
		t.Fatalf("new management service: %v", err)
	}
	return service
}

func mustReceiveEvent(t *testing.T, events <-chan *pb.SyncConfigEvent) *pb.SyncConfigEvent {
	t.Helper()

	select {
	case event := <-events:
		return event
	case <-time.After(2 * time.Second):
		t.Fatal("timed out waiting for sync config event")
		return nil
	}
}

type syncConfigStream struct {
	ctx    context.Context
	events chan *pb.SyncConfigEvent
}

func newSyncConfigStream(ctx context.Context) *syncConfigStream {
	return &syncConfigStream{
		ctx:    ctx,
		events: make(chan *pb.SyncConfigEvent, 8),
	}
}

func (s *syncConfigStream) Context() context.Context {
	return s.ctx
}

func (s *syncConfigStream) Send(event *pb.SyncConfigEvent) error {
	s.events <- event
	return nil
}

func (s *syncConfigStream) SetHeader(metadata.MD) error {
	return nil
}

func (s *syncConfigStream) SendHeader(metadata.MD) error {
	return nil
}

func (s *syncConfigStream) SetTrailer(metadata.MD) {}

func (s *syncConfigStream) SendMsg(any) error {
	return nil
}

func (s *syncConfigStream) RecvMsg(any) error {
	return nil
}
