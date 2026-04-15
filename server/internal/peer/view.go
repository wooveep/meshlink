package peer

import (
	"fmt"
	"sort"

	"meshlink/server/internal/device"
	"meshlink/server/pkg/pb"
)

// BuildVisiblePeers returns the complete visible peer set for a device.
func BuildVisiblePeers(selfID string, records []*device.Record, hooks ...AllowedIPsHook) []*pb.Peer {
	self := findRecord(selfID, records)
	if len(hooks) == 0 {
		hooks = DefaultAllowedIPsHooks()
	}

	peers := make([]*pb.Peer, 0, len(records))
	for _, record := range records {
		if record.ID == selfID {
			continue
		}

		visible := &pb.Peer{
			PeerId:    record.ID,
			PublicKey: record.PublicKey,
			Overlay: &pb.OverlayAddress{
				Ipv4: record.OverlayIP,
			},
			PreferredPath:  pb.PathType_PATH_TYPE_UNSPECIFIED,
			DirectEndpoint: toPBDirectEndpoint(record.DirectEndpoint),
		}

		if err := applyAllowedIPsHooks(hooks, self, record, visible); err != nil {
			panic(fmt.Sprintf("build visible peers: %v", err))
		}

		peers = append(peers, visible)
	}

	sort.Slice(peers, func(i, j int) bool {
		return peers[i].GetPeerId() < peers[j].GetPeerId()
	})

	return peers
}

func applyAllowedIPsHooks(hooks []AllowedIPsHook, self *device.Record, peerRecord *device.Record, visible *pb.Peer) error {
	for _, hook := range hooks {
		if err := hook.Apply(self, peerRecord, visible); err != nil {
			return fmt.Errorf("%s: %w", hook.Name(), err)
		}
	}
	return nil
}

func findRecord(id string, records []*device.Record) *device.Record {
	for _, record := range records {
		if record.ID == id {
			return record
		}
	}
	return nil
}

func toPBDirectEndpoint(endpoint *device.DirectEndpoint) *pb.DirectEndpoint {
	if endpoint == nil {
		return nil
	}

	return &pb.DirectEndpoint{
		Host: endpoint.Host,
		Port: endpoint.Port,
	}
}
