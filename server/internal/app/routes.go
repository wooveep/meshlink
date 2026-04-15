package app

import (
	"fmt"
	"net"
	"sort"
	"strings"

	"meshlink/server/internal/device"
)

func normalizeAdvertisedRoutes(routes []string) ([]string, error) {
	normalized := make(map[string]struct{}, len(routes))
	for _, raw := range routes {
		trimmed := strings.TrimSpace(raw)
		if trimmed == "" {
			continue
		}

		_, network, err := net.ParseCIDR(trimmed)
		if err != nil {
			return nil, fmt.Errorf("invalid CIDR %q: %w", raw, err)
		}
		if network.IP.To4() == nil {
			return nil, fmt.Errorf("only IPv4 advertised routes are supported: %s", network.String())
		}

		prefix, bits := network.Mask.Size()
		if bits != 32 {
			return nil, fmt.Errorf("only IPv4 advertised routes are supported: %s", network.String())
		}
		if prefix == 0 {
			return nil, fmt.Errorf("default route advertisement is not allowed: %s", network.String())
		}

		normalized[network.String()] = struct{}{}
	}

	result := make([]string, 0, len(normalized))
	for route := range normalized {
		result = append(result, route)
	}
	sort.Strings(result)
	return result, nil
}

func validateAdvertisedRoutes(
	routes []string,
	overlayNetwork *net.IPNet,
	existing []*device.Record,
	selfPublicKey string,
) error {
	for _, route := range routes {
		_, network, err := net.ParseCIDR(route)
		if err != nil {
			return fmt.Errorf("parse advertised route %s: %w", route, err)
		}
		if cidrOverlaps(network, overlayNetwork) {
			return fmt.Errorf("advertised route overlaps overlay network: %s", route)
		}
		for _, record := range existing {
			if record.PublicKey == selfPublicKey {
				continue
			}
			for _, existingRoute := range record.AdvertisedRoutes {
				_, existingNetwork, err := net.ParseCIDR(existingRoute)
				if err != nil {
					return fmt.Errorf("parse existing advertised route %s: %w", existingRoute, err)
				}
				if cidrOverlaps(network, existingNetwork) {
					return fmt.Errorf("advertised route overlaps route from device %s: %s", record.ID, route)
				}
			}
		}
	}

	return nil
}

func cidrOverlaps(left *net.IPNet, right *net.IPNet) bool {
	return left.Contains(right.IP) || right.Contains(left.IP)
}
