package peer

import (
	"sort"

	"meshlink/server/internal/device"
	"meshlink/server/pkg/pb"
)

// BuildVisiblePeers returns the complete visible peer set for a device.
func BuildVisiblePeers(selfID string, records []*device.Record) []*pb.Peer {
	peers := make([]*pb.Peer, 0, len(records))
	for _, record := range records {
		if record.ID == selfID {
			continue
		}

		peers = append(peers, &pb.Peer{
			PeerId:    record.ID,
			PublicKey: record.PublicKey,
			Overlay: &pb.OverlayAddress{
				Ipv4: record.OverlayIP,
			},
			AllowedIps: []string{
				record.OverlayIP + "/32",
			},
			PreferredPath:  pb.PathType_PATH_TYPE_UNSPECIFIED,
			DirectEndpoint: toPBDirectEndpoint(record.DirectEndpoint),
		})
	}

	sort.Slice(peers, func(i, j int) bool {
		return peers[i].GetPeerId() < peers[j].GetPeerId()
	})

	return peers
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
