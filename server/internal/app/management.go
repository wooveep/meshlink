package app

import (
	"context"
	"fmt"

	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"

	"meshlink/server/internal/auth"
	"meshlink/server/internal/device"
	"meshlink/server/internal/ipam"
	"meshlink/server/internal/peer"
	"meshlink/server/pkg/pb"
	"time"
)

type ManagementConfig struct {
	BootstrapToken string
	OverlayCIDR    string
	SyncInterval   time.Duration
}

type ManagementService struct {
	pb.UnimplementedManagementServiceServer

	auth         *auth.TokenValidator
	registry     *device.Registry
	allocator    *ipam.Allocator
	syncInterval time.Duration
}

func NewManagementService(cfg ManagementConfig) (*ManagementService, error) {
	allocator, err := ipam.NewAllocator(cfg.OverlayCIDR)
	if err != nil {
		return nil, fmt.Errorf("new allocator: %w", err)
	}

	return &ManagementService{
		auth:         auth.NewTokenValidator(cfg.BootstrapToken),
		registry:     device.NewRegistry(),
		allocator:    allocator,
		syncInterval: cfg.SyncInterval,
	}, nil
}

func (s *ManagementService) RegisterDevice(ctx context.Context, req *pb.RegisterDeviceRequest) (*pb.RegisterDeviceResponse, error) {
	if req.GetName() == "" {
		return nil, status.Error(codes.InvalidArgument, "name is required")
	}
	if req.GetPublicKey() == "" {
		return nil, status.Error(codes.InvalidArgument, "public_key is required")
	}
	if err := s.auth.Validate(req.GetToken()); err != nil {
		return nil, status.Error(codes.Unauthenticated, err.Error())
	}

	overlayIP, err := s.allocator.Allocate(req.GetPublicKey())
	if err != nil {
		return nil, status.Error(codes.ResourceExhausted, err.Error())
	}

	record := s.registry.Register(device.Registration{
		Name:      req.GetName(),
		PublicKey: req.GetPublicKey(),
		OS:        req.GetOs(),
		Version:   req.GetVersion(),
		OverlayIP: overlayIP,
	})

	return &pb.RegisterDeviceResponse{
		Device: toPBDevice(record),
		Peers:  s.visiblePeers(record.ID),
	}, nil
}

func (s *ManagementService) SyncConfig(req *pb.SyncConfigRequest, stream pb.ManagementService_SyncConfigServer) error {
	if _, ok := s.registry.GetByID(req.GetDeviceId()); !ok {
		return status.Error(codes.NotFound, "device not found")
	}

	updates, cancel := s.registry.Subscribe()
	defer cancel()

	send := func(eventType pb.SyncConfigEventType) error {
		record, ok := s.registry.GetByID(req.GetDeviceId())
		if !ok {
			return status.Error(codes.NotFound, "device not found")
		}
		return stream.Send(&pb.SyncConfigEvent{
			Type:     eventType,
			Self:     toPBDevice(record),
			Peers:    s.visiblePeers(record.ID),
			Revision: s.registry.CurrentRevision(),
		})
	}

	if err := send(pb.SyncConfigEventType_SYNC_CONFIG_EVENT_TYPE_FULL); err != nil {
		return err
	}

	ticker := time.NewTicker(s.syncInterval)
	defer ticker.Stop()

	for {
		select {
		case <-stream.Context().Done():
			return nil
		case <-updates:
			if err := send(pb.SyncConfigEventType_SYNC_CONFIG_EVENT_TYPE_INCREMENTAL); err != nil {
				return err
			}
		case <-ticker.C:
			if err := send(pb.SyncConfigEventType_SYNC_CONFIG_EVENT_TYPE_INCREMENTAL); err != nil {
				return err
			}
		}
	}
}

func (s *ManagementService) visiblePeers(selfID string) []*pb.Peer {
	return peer.BuildVisiblePeers(selfID, s.registry.List())
}

func (s *ManagementService) GetDevice(ctx context.Context, req *pb.GetDeviceRequest) (*pb.GetDeviceResponse, error) {
	record, ok := s.registry.GetByID(req.GetDeviceId())
	if !ok {
		return nil, status.Error(codes.NotFound, "device not found")
	}
	return &pb.GetDeviceResponse{Device: toPBDevice(record)}, nil
}

func toPBDevice(record *device.Record) *pb.Device {
	return &pb.Device{
		Id:        record.ID,
		Name:      record.Name,
		PublicKey: record.PublicKey,
		Version:   record.Version,
		Os:        record.OS,
		Overlay: &pb.OverlayAddress{
			Ipv4: record.OverlayIP,
		},
		Labels: map[string]string{},
	}
}
