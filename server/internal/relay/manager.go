package relay

import (
	"fmt"
	"net"
	"sort"
	"sync"
	"time"
)

type ManagerConfig struct {
	SessionTTL    time.Duration
	AdvertiseHost string
	UDPBindHost   string
}

type Manager struct {
	mu            sync.Mutex
	sessionTTL    time.Duration
	advertiseHost string
	udpBindHost   string
	sessions      map[string]*Session
}

func NewManager(cfg ManagerConfig) (*Manager, error) {
	if cfg.SessionTTL <= 0 {
		cfg.SessionTTL = 30 * time.Second
	}
	if cfg.AdvertiseHost == "" {
		cfg.AdvertiseHost = "127.0.0.1"
	}
	if cfg.UDPBindHost == "" {
		cfg.UDPBindHost = "0.0.0.0"
	}

	return &Manager{
		sessionTTL:    cfg.SessionTTL,
		advertiseHost: cfg.AdvertiseHost,
		udpBindHost:   cfg.UDPBindHost,
		sessions:      make(map[string]*Session),
	}, nil
}

func (m *Manager) Reserve(deviceID, peerID string, now time.Time) (*Session, error) {
	key := sessionKey(deviceID, peerID)

	m.mu.Lock()
	defer m.mu.Unlock()

	session, ok := m.sessions[key]
	if !ok {
		udpAddr, err := net.ResolveUDPAddr("udp", net.JoinHostPort(m.udpBindHost, "0"))
		if err != nil {
			return nil, fmt.Errorf("resolve udp bind host: %w", err)
		}
		conn, err := net.ListenUDP("udp", udpAddr)
		if err != nil {
			return nil, fmt.Errorf("listen udp relay socket: %w", err)
		}

		session = newSession(key, m.advertiseHost, conn, deviceID, peerID)
		m.sessions[key] = session
	}

	session.reserve(deviceID, now.Add(m.sessionTTL))
	return session, nil
}

func (m *Manager) Release(deviceID, peerID, sessionID string, now time.Time) error {
	key := sessionKey(deviceID, peerID)

	m.mu.Lock()
	defer m.mu.Unlock()

	session, ok := m.sessions[key]
	if !ok {
		return fmt.Errorf("relay session not found")
	}
	if session.ID() != sessionID {
		return fmt.Errorf("relay session id mismatch")
	}
	if !session.hasMember(deviceID) {
		return fmt.Errorf("device is not a member of relay session")
	}

	session.release(deviceID)
	if session.expired(now) {
		session.close()
		delete(m.sessions, key)
	}

	return nil
}

func (m *Manager) ReapExpired(now time.Time) []string {
	m.mu.Lock()
	defer m.mu.Unlock()

	var expired []string
	for key, session := range m.sessions {
		session.reap(now)
		if session.expired(now) {
			expired = append(expired, session.ID())
			session.close()
			delete(m.sessions, key)
		}
	}
	sort.Strings(expired)
	return expired
}

func sessionKey(left, right string) string {
	parts := []string{left, right}
	sort.Strings(parts)
	return fmt.Sprintf("%s::%s", parts[0], parts[1])
}
