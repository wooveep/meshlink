package relay

import (
	"context"
	"fmt"
	"log"
	"net"
	"strings"
	"time"

	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"

	"meshlink/server/internal/auth"
	"meshlink/server/pkg/pb"
)

type DeviceLookup interface {
	GetDevice(context.Context, *pb.GetDeviceRequest) (*pb.GetDeviceResponse, error)
}

type ServiceConfig struct {
	BootstrapToken string
	SessionTTL     time.Duration
	AdvertiseHost  string
	UDPBindHost    string
	DeviceLookup   DeviceLookup
}

type Service struct {
	pb.UnimplementedRelayServiceServer

	auth          *auth.TokenValidator
	sessionTTL    time.Duration
	advertiseHost string
	devices       DeviceLookup
	manager       *Manager
}

func NewService(cfg ServiceConfig) (*Service, error) {
	if cfg.SessionTTL <= 0 {
		cfg.SessionTTL = 30 * time.Second
	}

	manager, err := NewManager(ManagerConfig{
		SessionTTL:    cfg.SessionTTL,
		AdvertiseHost: cfg.AdvertiseHost,
		UDPBindHost:   cfg.UDPBindHost,
	})
	if err != nil {
		return nil, fmt.Errorf("new relay manager: %w", err)
	}

	return &Service{
		auth:          auth.NewTokenValidator(cfg.BootstrapToken),
		sessionTTL:    cfg.SessionTTL,
		advertiseHost: cfg.AdvertiseHost,
		devices:       cfg.DeviceLookup,
		manager:       manager,
	}, nil
}

func (s *Service) Manager() *Manager {
	return s.manager
}

func (s *Service) RunCleanup(ctx context.Context, interval time.Duration) {
	if interval <= 0 {
		interval = time.Second
	}

	ticker := time.NewTicker(interval)
	defer ticker.Stop()

	for {
		select {
		case <-ctx.Done():
			return
		case now := <-ticker.C:
			s.manager.ReapExpired(now)
		}
	}
}

func (s *Service) ReservePeerRelay(ctx context.Context, req *pb.ReservePeerRelayRequest) (*pb.ReservePeerRelayResponse, error) {
	if req.GetPeerId() == "" {
		return nil, status.Error(codes.InvalidArgument, "peer_id is required")
	}
	if err := s.validateCaller(ctx, req.GetDeviceId(), req.GetPublicKey(), req.GetBootstrapToken()); err != nil {
		return nil, err
	}
	if err := s.validatePeerExists(ctx, req.GetPeerId()); err != nil {
		return nil, err
	}

	session, err := s.manager.Reserve(req.GetDeviceId(), req.GetPeerId(), time.Now())
	if err != nil {
		return nil, status.Errorf(codes.Internal, "reserve relay session: %v", err)
	}
	log.Printf(
		"relay reservation active session=%s device=%s peer=%s udp_port=%d",
		session.ID(),
		req.GetDeviceId(),
		req.GetPeerId(),
		session.Port(),
	)

	return &pb.ReservePeerRelayResponse{
		RelayHost:  session.AdvertiseHost(),
		UdpPort:    uint32(session.Port()),
		TtlSeconds: uint32(s.sessionTTL / time.Second),
		SessionId:  session.ID(),
	}, nil
}

func (s *Service) ReleasePeerRelay(ctx context.Context, req *pb.ReleasePeerRelayRequest) (*pb.ReleasePeerRelayResponse, error) {
	if req.GetPeerId() == "" {
		return nil, status.Error(codes.InvalidArgument, "peer_id is required")
	}
	if req.GetSessionId() == "" {
		return nil, status.Error(codes.InvalidArgument, "session_id is required")
	}
	if err := s.validateCaller(ctx, req.GetDeviceId(), req.GetPublicKey(), req.GetBootstrapToken()); err != nil {
		return nil, err
	}

	if err := s.manager.Release(req.GetDeviceId(), req.GetPeerId(), req.GetSessionId(), time.Now()); err != nil {
		if strings.Contains(err.Error(), "not found") {
			return nil, status.Error(codes.NotFound, err.Error())
		}
		if strings.Contains(err.Error(), "mismatch") || strings.Contains(err.Error(), "not a member") {
			return nil, status.Error(codes.PermissionDenied, err.Error())
		}
		return nil, status.Errorf(codes.Internal, "release relay session: %v", err)
	}
	log.Printf(
		"relay reservation released session=%s device=%s peer=%s reason=%s",
		req.GetSessionId(),
		req.GetDeviceId(),
		req.GetPeerId(),
		req.GetReason(),
	)

	return &pb.ReleasePeerRelayResponse{}, nil
}

func (s *Service) validateCaller(ctx context.Context, deviceID, publicKey, bootstrapToken string) error {
	if deviceID == "" {
		return status.Error(codes.InvalidArgument, "device_id is required")
	}
	if publicKey == "" {
		return status.Error(codes.InvalidArgument, "public_key is required")
	}
	if err := s.auth.Validate(bootstrapToken); err != nil {
		return status.Error(codes.Unauthenticated, err.Error())
	}
	if s.devices == nil {
		return nil
	}

	response, err := s.devices.GetDevice(ctx, &pb.GetDeviceRequest{DeviceId: deviceID})
	if err != nil {
		return status.Errorf(codes.Unauthenticated, "lookup device %s: %v", deviceID, err)
	}
	if response.GetDevice() == nil {
		return status.Error(codes.Unauthenticated, "device lookup returned empty record")
	}
	if response.GetDevice().GetPublicKey() != publicKey {
		return status.Error(codes.Unauthenticated, "device public key mismatch")
	}
	return nil
}

func (s *Service) validatePeerExists(ctx context.Context, peerID string) error {
	if s.devices == nil {
		return nil
	}

	response, err := s.devices.GetDevice(ctx, &pb.GetDeviceRequest{DeviceId: peerID})
	if err != nil {
		return status.Errorf(codes.NotFound, "lookup peer %s: %v", peerID, err)
	}
	if response.GetDevice() == nil {
		return status.Error(codes.NotFound, "peer lookup returned empty record")
	}
	return nil
}

func ResolveAdvertiseHost(listenAddr, explicitHost string) string {
	if explicitHost != "" {
		return explicitHost
	}

	host, _, err := net.SplitHostPort(listenAddr)
	if err != nil {
		return "127.0.0.1"
	}
	switch host {
	case "", "0.0.0.0", "::", "[::]":
		return "127.0.0.1"
	default:
		return strings.Trim(host, "[]")
	}
}
