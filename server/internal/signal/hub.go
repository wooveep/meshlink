package signal

import (
	"sync"
	"time"

	"meshlink/server/pkg/pb"
)

type Session struct {
	deviceID string
	outbound chan *pb.SignalEnvelope
	closed   chan struct{}

	mu       sync.RWMutex
	lastSeen time.Time
}

func NewSession(deviceID string, now time.Time) *Session {
	return &Session{
		deviceID: deviceID,
		outbound: make(chan *pb.SignalEnvelope, 32),
		closed:   make(chan struct{}),
		lastSeen: now,
	}
}

func (s *Session) DeviceID() string {
	return s.deviceID
}

func (s *Session) Outbound() <-chan *pb.SignalEnvelope {
	return s.outbound
}

func (s *Session) Touch(now time.Time) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.lastSeen = now
}

func (s *Session) LastSeen() time.Time {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return s.lastSeen
}

func (s *Session) Closed() <-chan struct{} {
	return s.closed
}

func (s *Session) Close() {
	select {
	case <-s.closed:
		return
	default:
		close(s.closed)
		close(s.outbound)
	}
}

type Hub struct {
	mu       sync.RWMutex
	sessions map[string]*Session
}

func NewHub() *Hub {
	return &Hub{
		sessions: make(map[string]*Session),
	}
}

func (h *Hub) Register(session *Session) (replaced *Session) {
	h.mu.Lock()
	defer h.mu.Unlock()

	if existing, ok := h.sessions[session.DeviceID()]; ok && existing != session {
		replaced = existing
		existing.Close()
	}

	h.sessions[session.DeviceID()] = session
	return replaced
}

func (h *Hub) Remove(session *Session) {
	h.mu.Lock()
	defer h.mu.Unlock()

	current, ok := h.sessions[session.DeviceID()]
	if !ok || current != session {
		return
	}

	delete(h.sessions, session.DeviceID())
	session.Close()
}

func (h *Hub) Route(targetDeviceID string, envelope *pb.SignalEnvelope) bool {
	h.mu.RLock()
	session, ok := h.sessions[targetDeviceID]
	h.mu.RUnlock()
	if !ok {
		return false
	}

	select {
	case <-session.Closed():
		return false
	default:
	}

	select {
	case session.outbound <- envelope:
		return true
	default:
		select {
		case <-session.outbound:
		default:
		}
		select {
		case session.outbound <- envelope:
			return true
		default:
			return false
		}
	}
}

func (h *Hub) ReapExpired(now time.Time, timeout time.Duration) []string {
	h.mu.Lock()
	defer h.mu.Unlock()

	var expired []string
	for deviceID, session := range h.sessions {
		if timeout > 0 && now.Sub(session.LastSeen()) > timeout {
			expired = append(expired, deviceID)
			delete(h.sessions, deviceID)
			session.Close()
		}
	}

	return expired
}
