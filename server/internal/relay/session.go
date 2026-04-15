package relay

import (
	"net"
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
	learned map[string]*net.UDPAddr
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
		learned: make(map[string]*net.UDPAddr),
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

		target := s.learnSource(sourceAddr)
		if target == nil {
			continue
		}
		_, _ = s.conn.WriteToUDP(buffer[:n], target)
	}
}

func (s *Session) learnSource(sourceAddr *net.UDPAddr) *net.UDPAddr {
	sourceKey := sourceAddr.String()

	s.mu.Lock()
	defer s.mu.Unlock()

	if _, ok := s.learned[sourceKey]; !ok {
		if len(s.learned) >= 2 {
			return nil
		}
		s.learned[sourceKey] = copyUDPAddr(sourceAddr)
	}

	for key, target := range s.learned {
		if key != sourceKey {
			return copyUDPAddr(target)
		}
	}

	return nil
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
