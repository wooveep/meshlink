package signal

import (
	"context"
	"errors"
	"fmt"
	"io"
	"time"

	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"

	"meshlink/server/internal/auth"
	"meshlink/server/pkg/pb"
)

type DeviceLookup interface {
	GetDevice(context.Context, *pb.GetDeviceRequest) (*pb.GetDeviceResponse, error)
}

type ServiceConfig struct {
	BootstrapToken   string
	HeartbeatTimeout time.Duration
	DeviceLookup     DeviceLookup
}

type Service struct {
	pb.UnimplementedSignalServiceServer

	auth             *auth.TokenValidator
	heartbeatTimeout time.Duration
	devices          DeviceLookup
	hub              *Hub
}

func NewService(cfg ServiceConfig) *Service {
	return &Service{
		auth:             auth.NewTokenValidator(cfg.BootstrapToken),
		heartbeatTimeout: cfg.HeartbeatTimeout,
		devices:          cfg.DeviceLookup,
		hub:              NewHub(),
	}
}

func (s *Service) Hub() *Hub {
	return s.hub
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
			s.hub.ReapExpired(now, s.heartbeatTimeout)
		}
	}
}

func (s *Service) OpenSignal(stream pb.SignalService_OpenSignalServer) error {
	if err := stream.SendHeader(metadata.MD{}); err != nil {
		return status.Errorf(codes.Internal, "send signal headers: %v", err)
	}

	first, err := stream.Recv()
	if err != nil {
		if errors.Is(err, io.EOF) {
			return status.Error(codes.InvalidArgument, "signal stream closed before hello")
		}
		return err
	}

	hello := first.GetHello()
	if first.GetKind() != pb.SignalKind_SIGNAL_KIND_HELLO || hello == nil {
		return status.Error(codes.InvalidArgument, "first signal frame must be hello")
	}

	deviceID := hello.GetDeviceId()
	if err := s.validateHello(stream.Context(), hello); err != nil {
		return err
	}

	session := NewSession(deviceID, time.Now())
	s.hub.Register(session)
	defer s.hub.Remove(session)

	errCh := make(chan error, 1)
	go func() {
		for {
			select {
			case <-stream.Context().Done():
				errCh <- nil
				return
			case <-session.Closed():
				errCh <- status.Error(codes.Aborted, "signal session replaced or expired")
				return
			case outbound, ok := <-session.Outbound():
				if !ok {
					errCh <- nil
					return
				}
				if err := stream.Send(outbound); err != nil {
					errCh <- err
					return
				}
			}
		}
	}()

	for {
		select {
		case <-session.Closed():
			return status.Error(codes.Aborted, "signal session replaced or expired")
		case err := <-errCh:
			return err
		default:
		}

		envelope, err := stream.Recv()
		if err != nil {
			if errors.Is(err, io.EOF) {
				return nil
			}
			return err
		}

		session.Touch(time.Now())

		if err := s.handleEnvelope(session, envelope); err != nil {
			return err
		}
	}
}

func (s *Service) validateHello(ctx context.Context, hello *pb.SignalHello) error {
	if hello.GetDeviceId() == "" {
		return status.Error(codes.InvalidArgument, "hello.device_id is required")
	}
	if hello.GetPublicKey() == "" {
		return status.Error(codes.InvalidArgument, "hello.public_key is required")
	}
	if err := s.auth.Validate(hello.GetBootstrapToken()); err != nil {
		return status.Error(codes.Unauthenticated, err.Error())
	}
	if s.devices == nil {
		return nil
	}

	response, err := s.devices.GetDevice(ctx, &pb.GetDeviceRequest{DeviceId: hello.GetDeviceId()})
	if err != nil {
		return status.Errorf(codes.Unauthenticated, "lookup device %s: %v", hello.GetDeviceId(), err)
	}
	if response.GetDevice() == nil {
		return status.Error(codes.Unauthenticated, "device lookup returned empty record")
	}
	if response.GetDevice().GetPublicKey() != hello.GetPublicKey() {
		return status.Error(codes.Unauthenticated, "device public key mismatch")
	}

	return nil
}

func (s *Service) handleEnvelope(session *Session, envelope *pb.SignalEnvelope) error {
	switch envelope.GetKind() {
	case pb.SignalKind_SIGNAL_KIND_HEARTBEAT:
		return nil
	case pb.SignalKind_SIGNAL_KIND_CANDIDATES,
		pb.SignalKind_SIGNAL_KIND_PUNCH_REQUEST,
		pb.SignalKind_SIGNAL_KIND_PUNCH_RESULT:
		if envelope.GetTargetDeviceId() == "" {
			return status.Error(codes.InvalidArgument, "target_device_id is required")
		}

		forwarded := &pb.SignalEnvelope{
			Kind:           envelope.GetKind(),
			SourceDeviceId: session.DeviceID(),
			TargetDeviceId: envelope.GetTargetDeviceId(),
			SessionId:      envelope.GetSessionId(),
		}

		switch body := envelope.Body.(type) {
		case *pb.SignalEnvelope_CandidateAnnouncement:
			forwarded.Body = &pb.SignalEnvelope_CandidateAnnouncement{
				CandidateAnnouncement: body.CandidateAnnouncement,
			}
		case *pb.SignalEnvelope_PunchRequest:
			forwarded.Body = &pb.SignalEnvelope_PunchRequest{
				PunchRequest: body.PunchRequest,
			}
		case *pb.SignalEnvelope_PunchResult:
			forwarded.Body = &pb.SignalEnvelope_PunchResult{
				PunchResult: body.PunchResult,
			}
		default:
			return status.Error(codes.InvalidArgument, "signal body does not match kind")
		}

		_ = s.hub.Route(envelope.GetTargetDeviceId(), forwarded)
		return nil
	default:
		return status.Errorf(codes.InvalidArgument, "unsupported signal kind: %s", envelope.GetKind())
	}
}

type ManagementDeviceLookup struct {
	client pb.ManagementServiceClient
}

func NewManagementDeviceLookup(client pb.ManagementServiceClient) *ManagementDeviceLookup {
	return &ManagementDeviceLookup{client: client}
}

func (l *ManagementDeviceLookup) GetDevice(ctx context.Context, req *pb.GetDeviceRequest) (*pb.GetDeviceResponse, error) {
	if l == nil || l.client == nil {
		return nil, fmt.Errorf("management client is not configured")
	}
	return l.client.GetDevice(ctx, req)
}
