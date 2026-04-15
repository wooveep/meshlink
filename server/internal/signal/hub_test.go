package signal

import (
	"context"
	"testing"
	"time"

	"meshlink/server/pkg/pb"
)

func TestHubRegisterReplacesExistingSession(t *testing.T) {
	hub := NewHub()
	first := NewSession("dev-a", time.Unix(10, 0))
	second := NewSession("dev-a", time.Unix(11, 0))

	replaced := hub.Register(first)
	if replaced != nil {
		t.Fatal("expected first registration not to replace a session")
	}

	replaced = hub.Register(second)
	if replaced != first {
		t.Fatal("expected second registration to replace first session")
	}

	select {
	case <-first.Closed():
	default:
		t.Fatal("expected replaced session to be closed")
	}
}

func TestHubRouteForwardsEnvelope(t *testing.T) {
	hub := NewHub()
	session := NewSession("dev-b", time.Now())
	hub.Register(session)

	envelope := &pb.SignalEnvelope{
		Kind:           pb.SignalKind_SIGNAL_KIND_CANDIDATES,
		SourceDeviceId: "dev-a",
		TargetDeviceId: "dev-b",
	}

	if !hub.Route("dev-b", envelope) {
		t.Fatal("expected route to succeed")
	}

	select {
	case got := <-session.Outbound():
		if got.GetSourceDeviceId() != "dev-a" {
			t.Fatalf("unexpected source device: %s", got.GetSourceDeviceId())
		}
	case <-time.After(time.Second):
		t.Fatal("timed out waiting for routed envelope")
	}
}

func TestHubReapExpiredSessions(t *testing.T) {
	hub := NewHub()
	session := NewSession("dev-a", time.Unix(0, 0))
	hub.Register(session)

	expired := hub.ReapExpired(time.Unix(10, 0), 5*time.Second)
	if len(expired) != 1 || expired[0] != "dev-a" {
		t.Fatalf("unexpected expired set: %#v", expired)
	}

	select {
	case <-session.Closed():
	default:
		t.Fatal("expected expired session to be closed")
	}
}

func TestServiceValidateHelloChecksDeviceIdentity(t *testing.T) {
	service := NewService(ServiceConfig{
		BootstrapToken:   "meshlink-dev-token",
		HeartbeatTimeout: 5 * time.Second,
		DeviceLookup: &stubDeviceLookup{response: &pb.GetDeviceResponse{
			Device: &pb.Device{Id: "dev-a", PublicKey: "pk-a"},
		}},
	})

	if err := service.validateHello(context.Background(), &pb.SignalHello{
		DeviceId:       "dev-a",
		PublicKey:      "pk-a",
		BootstrapToken: "meshlink-dev-token",
	}); err != nil {
		t.Fatalf("expected hello validation to succeed: %v", err)
	}

	if err := service.validateHello(context.Background(), &pb.SignalHello{
		DeviceId:       "dev-a",
		PublicKey:      "pk-bad",
		BootstrapToken: "meshlink-dev-token",
	}); err == nil {
		t.Fatal("expected public key mismatch to be rejected")
	}
}

type stubDeviceLookup struct {
	response *pb.GetDeviceResponse
	err      error
}

func (s *stubDeviceLookup) GetDevice(context.Context, *pb.GetDeviceRequest) (*pb.GetDeviceResponse, error) {
	if s.err != nil {
		return nil, s.err
	}
	return s.response, nil
}
