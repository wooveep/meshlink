package peer

import (
	"fmt"

	"meshlink/server/internal/device"
	"meshlink/server/pkg/pb"
)

type AllowedIPsHook interface {
	Name() string
	Apply(self *device.Record, peer *device.Record, visible *pb.Peer) error
}

type StaticRouteAdvertiserHook struct{}

func (StaticRouteAdvertiserHook) Name() string {
	return "static_route_advertiser"
}

func (StaticRouteAdvertiserHook) Apply(_ *device.Record, peer *device.Record, visible *pb.Peer) error {
	if peer == nil || visible == nil {
		return fmt.Errorf("peer hook received nil input")
	}

	allowedIPs := make([]string, 0, 1+len(peer.AdvertisedRoutes))
	if peer.OverlayIP != "" {
		allowedIPs = append(allowedIPs, peer.OverlayIP+"/32")
	}
	allowedIPs = append(allowedIPs, peer.AdvertisedRoutes...)
	visible.AllowedIps = allowedIPs
	return nil
}

func DefaultAllowedIPsHooks() []AllowedIPsHook {
	return []AllowedIPsHook{
		StaticRouteAdvertiserHook{},
	}
}
