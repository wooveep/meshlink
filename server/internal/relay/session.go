package relay

import (
	"net"
	"sort"
	"sync"
	"time"
)

type Session struct {
	id            string
	advertiseHost string
	conn          *net.UDPConn
	port          uint16
	members       map[string]time.Time

	mu      sync.RWMutex
	learned map[string]learnedEndpoint
}

type learnedEndpoint struct {
	addr     *net.UDPAddr
	lastSeen time.Time
}

func newSession(id, advertiseHost string, conn *net.UDPConn, left, right string) *Session {
	port := uint16(conn.LocalAddr().(*net.UDPAddr).Port)
	session := &Session{
		id:            id,
		advertiseHost: advertiseHost,
		conn:          conn,
		port:          port,
		members: map[string]time.Time{
			left:  time.Time{},
			right: time.Time{},
		},
		learned: make(map[string]learnedEndpoint),
	}

	go session.forwardLoop()
	return session
}

func (s *Session) ID() string {
	return s.id
}

func (s *Session) AdvertiseHost() string {
	return s.advertiseHost
}

func (s *Session) Port() uint16 {
	return s.port
}

func (s *Session) Members() []string {
	members := make([]string, 0, len(s.members))
	for member := range s.members {
		members = append(members, member)
	}
	sort.Strings(members)
	return members
}

func (s *Session) ExpiresAt() time.Time {
	var latest time.Time
	for _, expiresAt := range s.members {
		if expiresAt.After(latest) {
			latest = expiresAt
		}
	}
	return latest
}

func (s *Session) reserve(deviceID string, expiresAt time.Time) {
	s.members[deviceID] = expiresAt
}

func (s *Session) release(deviceID string) {
	s.members[deviceID] = time.Time{}
}

func (s *Session) hasMember(deviceID string) bool {
	_, ok := s.members[deviceID]
	return ok
}

func (s *Session) reap(now time.Time) {
	for deviceID, expiresAt := range s.members {
		if expiresAt.IsZero() {
			continue
		}
		if now.After(expiresAt) {
			s.members[deviceID] = time.Time{}
		}
	}
}

func (s *Session) expired(now time.Time) bool {
	for _, expiresAt := range s.members {
		if expiresAt.After(now) {
			return false
		}
	}
	return true
}

func (s *Session) close() {
	_ = s.conn.Close()
}

func (s *Session) forwardLoop() {
	buffer := make([]byte, 64*1024)
	for {
		n, sourceAddr, err := s.conn.ReadFromUDP(buffer)
		if err != nil {
			return
		}

		targets := s.learnSource(sourceAddr, time.Now())
		if len(targets) == 0 {
			continue
		}
		for _, target := range targets {
			_, _ = s.conn.WriteToUDP(buffer[:n], target)
		}
	}
}

func (s *Session) learnSource(sourceAddr *net.UDPAddr, now time.Time) []*net.UDPAddr {
	sourceKey := sourceAddr.String()

	s.mu.Lock()
	defer s.mu.Unlock()

	s.learned[sourceKey] = learnedEndpoint{
		addr:     copyUDPAddr(sourceAddr),
		lastSeen: now,
	}
	s.pruneLearnedLocked(sourceKey)

	targets := make([]*net.UDPAddr, 0, len(s.learned)-1)
	for key, endpoint := range s.learned {
		if key != sourceKey {
			targets = append(targets, copyUDPAddr(endpoint.addr))
		}
	}

	return targets
}

func (s *Session) pruneLearnedLocked(protectedKey string) {
	const maxLearnedEndpoints = 8
	for len(s.learned) > maxLearnedEndpoints {
		var oldestKey string
		var oldestTime time.Time
		for key, endpoint := range s.learned {
			if key == protectedKey {
				continue
			}
			if oldestKey == "" || endpoint.lastSeen.Before(oldestTime) {
				oldestKey = key
				oldestTime = endpoint.lastSeen
			}
		}
		if oldestKey == "" {
			return
		}
		delete(s.learned, oldestKey)
	}
}

func copyUDPAddr(addr *net.UDPAddr) *net.UDPAddr {
	if addr == nil {
		return nil
	}

	ip := make(net.IP, len(addr.IP))
	copy(ip, addr.IP)
	return &net.UDPAddr{
		IP:   ip,
		Port: addr.Port,
		Zone: addr.Zone,
	}
}
