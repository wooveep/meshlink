package relay

import (
	"context"
	"net"
	"testing"
	"time"

	"meshlink/server/pkg/pb"
)

type fakeDeviceLookup struct {
	devices map[string]*pb.Device
}

func (f *fakeDeviceLookup) GetDevice(_ context.Context, req *pb.GetDeviceRequest) (*pb.GetDeviceResponse, error) {
	return &pb.GetDeviceResponse{Device: f.devices[req.GetDeviceId()]}, nil
}

func TestManagerReserveReuseAndExpiry(t *testing.T) {
	manager, err := NewManager(ManagerConfig{
		SessionTTL:    2 * time.Second,
		AdvertiseHost: "198.51.100.10",
		UDPBindHost:   "127.0.0.1",
	})
	if err != nil {
		t.Fatalf("new manager: %v", err)
	}

	now := time.Unix(100, 0)
	first, err := manager.Reserve("dev-a", "dev-b", now)
	if err != nil {
		t.Fatalf("reserve first: %v", err)
	}
	second, err := manager.Reserve("dev-b", "dev-a", now.Add(time.Second))
	if err != nil {
		t.Fatalf("reserve second: %v", err)
	}

	if first.ID() != second.ID() {
		t.Fatalf("expected same session id, got %q and %q", first.ID(), second.ID())
	}
	if first.Port() != second.Port() {
		t.Fatalf("expected same UDP port, got %d and %d", first.Port(), second.Port())
	}

	expired := manager.ReapExpired(now.Add(4 * time.Second))
	if len(expired) != 1 || expired[0] != first.ID() {
		t.Fatalf("unexpected expired sessions: %#v", expired)
	}
}

func TestManagerReleaseClosesSession(t *testing.T) {
	manager, err := NewManager(ManagerConfig{
		SessionTTL:    10 * time.Second,
		AdvertiseHost: "198.51.100.10",
		UDPBindHost:   "127.0.0.1",
	})
	if err != nil {
		t.Fatalf("new manager: %v", err)
	}

	now := time.Unix(200, 0)
	session, err := manager.Reserve("dev-a", "dev-b", now)
	if err != nil {
		t.Fatalf("reserve session: %v", err)
	}
	if _, err := manager.Reserve("dev-b", "dev-a", now); err != nil {
		t.Fatalf("reserve peer session: %v", err)
	}

	if err := manager.Release("dev-a", "dev-b", session.ID(), now); err != nil {
		t.Fatalf("release dev-a: %v", err)
	}
	if err := manager.Release("dev-b", "dev-a", session.ID(), now); err != nil {
		t.Fatalf("release dev-b: %v", err)
	}

	if expired := manager.ReapExpired(now); len(expired) != 0 {
		t.Fatalf("expected no reap-needed sessions after explicit release, got %#v", expired)
	}
	if _, err := manager.Reserve("dev-a", "dev-b", now.Add(time.Second)); err != nil {
		t.Fatalf("reserve recreated session: %v", err)
	}
}

func TestRelaySessionForwardsPacketsAfterLearningBothPeers(t *testing.T) {
	manager, err := NewManager(ManagerConfig{
		SessionTTL:    10 * time.Second,
		AdvertiseHost: "127.0.0.1",
		UDPBindHost:   "127.0.0.1",
	})
	if err != nil {
		t.Fatalf("new manager: %v", err)
	}

	session, err := manager.Reserve("dev-a", "dev-b", time.Now())
	if err != nil {
		t.Fatalf("reserve session: %v", err)
	}
	if _, err := manager.Reserve("dev-b", "dev-a", time.Now()); err != nil {
		t.Fatalf("reserve peer session: %v", err)
	}

	left, err := net.ListenUDP("udp", &net.UDPAddr{IP: net.ParseIP("127.0.0.1"), Port: 0})
	if err != nil {
		t.Fatalf("listen left: %v", err)
	}
	defer left.Close()

	right, err := net.ListenUDP("udp", &net.UDPAddr{IP: net.ParseIP("127.0.0.1"), Port: 0})
	if err != nil {
		t.Fatalf("listen right: %v", err)
	}
	defer right.Close()

	target := &net.UDPAddr{IP: net.ParseIP("127.0.0.1"), Port: int(session.Port())}

	if _, err := left.WriteToUDP([]byte("left-hello"), target); err != nil {
		t.Fatalf("write left hello: %v", err)
	}
	if _, err := right.WriteToUDP([]byte("right-hello"), target); err != nil {
		t.Fatalf("write right hello: %v", err)
	}

	buffer := make([]byte, 64)
	if err := left.SetReadDeadline(time.Now().Add(2 * time.Second)); err != nil {
		t.Fatalf("set left deadline: %v", err)
	}
	n, _, err := left.ReadFromUDP(buffer)
	if err != nil {
		t.Fatalf("read forwarded packet on left: %v", err)
	}
	if got := string(buffer[:n]); got != "right-hello" {
		t.Fatalf("unexpected forwarded payload on left: %q", got)
	}

	if _, err := left.WriteToUDP([]byte("left-data"), target); err != nil {
		t.Fatalf("write left data: %v", err)
	}
	if err := right.SetReadDeadline(time.Now().Add(2 * time.Second)); err != nil {
		t.Fatalf("set right deadline: %v", err)
	}
	n, _, err = right.ReadFromUDP(buffer)
	if err != nil {
		t.Fatalf("read forwarded packet on right: %v", err)
	}
	if got := string(buffer[:n]); got != "left-data" {
		t.Fatalf("unexpected forwarded payload on right: %q", got)
	}
}

func TestServiceReserveAndReleaseValidateDeviceIdentity(t *testing.T) {
	service, err := NewService(ServiceConfig{
		BootstrapToken: "meshlink-dev-token",
		SessionTTL:     10 * time.Second,
		AdvertiseHost:  "198.51.100.10",
		UDPBindHost:    "127.0.0.1",
		DeviceLookup: &fakeDeviceLookup{
			devices: map[string]*pb.Device{
				"dev-a": {Id: "dev-a", PublicKey: "pk-a"},
				"dev-b": {Id: "dev-b", PublicKey: "pk-b"},
			},
		},
	})
	if err != nil {
		t.Fatalf("new service: %v", err)
	}

	reserve, err := service.ReservePeerRelay(context.Background(), &pb.ReservePeerRelayRequest{
		DeviceId:       "dev-a",
		PublicKey:      "pk-a",
		BootstrapToken: "meshlink-dev-token",
		PeerId:         "dev-b",
	})
	if err != nil {
		t.Fatalf("reserve relay: %v", err)
	}
	if reserve.GetRelayHost() != "198.51.100.10" {
		t.Fatalf("unexpected relay host: %q", reserve.GetRelayHost())
	}
	if reserve.GetUdpPort() == 0 {
		t.Fatalf("expected non-zero UDP port")
	}

	if _, err := service.ReleasePeerRelay(context.Background(), &pb.ReleasePeerRelayRequest{
		DeviceId:       "dev-a",
		PublicKey:      "wrong-pk",
		BootstrapToken: "meshlink-dev-token",
		PeerId:         "dev-b",
		SessionId:      reserve.GetSessionId(),
	}); err == nil {
		t.Fatalf("expected release validation error for mismatched public key")
	}
}
