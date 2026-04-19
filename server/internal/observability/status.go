package observability

import "time"

type ServiceStatusEnvelope struct {
	Service string      `json:"service"`
	Healthy bool        `json:"healthy"`
	Now     time.Time   `json:"now"`
	Status  interface{} `json:"status"`
}

type SignalSessionStatus struct {
	DeviceID string    `json:"device_id"`
	LastSeen time.Time `json:"last_seen"`
}

type SignalStatus struct {
	ActiveSessions   int                   `json:"active_sessions"`
	Sessions         []SignalSessionStatus `json:"sessions"`
	RouteHits        int64                 `json:"route_hits"`
	RouteMisses      int64                 `json:"route_misses"`
	ExpiredSessions  int64                 `json:"expired_sessions"`
	HeartbeatTimeout string                `json:"heartbeat_timeout"`
}

type RelaySessionStatus struct {
	SessionID string    `json:"session_id"`
	Port      uint16    `json:"port"`
	Members   []string  `json:"members"`
	ExpiresAt time.Time `json:"expires_at"`
}

type RelayStatus struct {
	ActiveSessions int                  `json:"active_sessions"`
	Sessions       []RelaySessionStatus `json:"sessions"`
	Reservations   int64                `json:"reservations"`
	ReapedSessions int64                `json:"reaped_sessions"`
	SessionTTL     string               `json:"session_ttl"`
	AdvertiseHost  string               `json:"advertise_host"`
}
