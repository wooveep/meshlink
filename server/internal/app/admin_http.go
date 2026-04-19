package app

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strings"
	"sync"
	"time"

	"meshlink/server/internal/adminui"
	"meshlink/server/internal/device"
	"meshlink/server/internal/observability"
	"meshlink/server/internal/store"
)

const adminSessionCookieName = "meshlink_admin_session"

type AdminHTTPConfig struct {
	AdminToken        string
	GRPCListenAddr    string
	HTTPListenAddr    string
	OverlayCIDR       string
	SyncInterval      time.Duration
	StateDBPath       string
	SignaldStatusURL  string
	RelaydStatusURL   string
}

type AdminHTTPServer struct {
	management   *ManagementService
	statusClient *observability.HTTPStatusClient
	config       AdminHTTPConfig
	sessions     *adminSessionManager
}

type adminSessionManager struct {
	mu       sync.Mutex
	sessions map[string]time.Time
	ttl      time.Duration
}

type adminServiceEnvelope struct {
	Service string          `json:"service"`
	Healthy bool            `json:"healthy"`
	Now     time.Time       `json:"now"`
	Error   string          `json:"error,omitempty"`
	Status  json.RawMessage `json:"status,omitempty"`
}

type adminOverviewResponse struct {
	GeneratedAt  time.Time               `json:"generated_at"`
	Management   adminManagementSummary  `json:"management"`
	Services     []adminServiceEnvelope  `json:"services"`
	RecentEvents []store.AuditEvent      `json:"recent_events"`
}

type adminManagementSummary struct {
	Revision       string `json:"revision"`
	DeviceTotal    int    `json:"device_total"`
	OnlineDevices  int    `json:"online_devices"`
	OfflineDevices int    `json:"offline_devices"`
}

type adminDeviceResponse struct {
	ID               string            `json:"id"`
	Name             string            `json:"name"`
	PublicKey        string            `json:"public_key"`
	OS               string            `json:"os"`
	Version          string            `json:"version"`
	OverlayIP        string            `json:"overlay_ip"`
	DirectEndpoint   *adminEndpoint    `json:"direct_endpoint,omitempty"`
	AdvertisedRoutes []string          `json:"advertised_routes"`
	Labels           map[string]string `json:"labels"`
	Disabled         bool              `json:"disabled"`
	LastSeen         time.Time         `json:"last_seen"`
	Online           bool              `json:"online"`
}

type adminDeviceDetailResponse struct {
	Device adminDeviceResponse `json:"device"`
	Peers  []adminPeerResponse `json:"peers"`
}

type adminPeerResponse struct {
	PeerID         string         `json:"peer_id"`
	PublicKey      string         `json:"public_key"`
	OverlayIP      string         `json:"overlay_ip"`
	AllowedIPs     []string       `json:"allowed_ips"`
	DirectEndpoint *adminEndpoint `json:"direct_endpoint,omitempty"`
}

type adminEndpoint struct {
	Host string `json:"host"`
	Port uint32 `json:"port"`
}

type adminDevicePatchRequest struct {
	Name             *string           `json:"name"`
	Labels           map[string]string `json:"labels"`
	AdvertisedRoutes []string          `json:"advertised_routes"`
}

type adminSessionLoginRequest struct {
	Token string `json:"token"`
}

type adminConfigSummary struct {
	GRPCListenAddr       string `json:"grpc_listen_addr"`
	HTTPListenAddr       string `json:"http_listen_addr"`
	OverlayCIDR          string `json:"overlay_cidr"`
	SyncInterval         string `json:"sync_interval"`
	StateDBPath          string `json:"state_db_path"`
	AdminTokenConfigured bool   `json:"admin_token_configured"`
	SignaldStatusURL     string `json:"signald_status_url"`
	RelaydStatusURL      string `json:"relayd_status_url"`
}

func NewAdminHTTPServer(management *ManagementService, cfg AdminHTTPConfig) *AdminHTTPServer {
	return &AdminHTTPServer{
		management:   management,
		statusClient: observability.NewHTTPStatusClient(2 * time.Second),
		config:       cfg,
		sessions: &adminSessionManager{
			sessions: make(map[string]time.Time),
			ttl:      12 * time.Hour,
		},
	}
}

func (s *AdminHTTPServer) Handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("/api/admin/v1/session/login", s.handleLogin)
	mux.HandleFunc("/api/admin/v1/session/logout", s.withAuth(s.handleLogout))
	mux.HandleFunc("/api/admin/v1/overview", s.withAuth(s.handleOverview))
	mux.HandleFunc("/api/admin/v1/devices", s.withAuth(s.handleDevices))
	mux.HandleFunc("/api/admin/v1/devices/", s.withAuth(s.handleDeviceByID))
	mux.HandleFunc("/api/admin/v1/services", s.withAuth(s.handleServices))
	mux.HandleFunc("/api/admin/v1/config", s.withAuth(s.handleConfig))
	mux.HandleFunc("/api/admin/v1/audit", s.withAuth(s.handleAudit))
	mux.Handle("/admin/", adminui.Handler("/admin/"))
	mux.Handle("/", http.RedirectHandler("/admin/", http.StatusTemporaryRedirect))
	return mux
}

func (s *AdminHTTPServer) withAuth(next http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		if !s.sessions.Validate(r) {
			writeJSON(w, http.StatusUnauthorized, map[string]string{"error": "unauthorized"})
			return
		}
		next(w, r)
	}
}

func (s *AdminHTTPServer) handleLogin(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		writeJSON(w, http.StatusMethodNotAllowed, map[string]string{"error": "method not allowed"})
		return
	}
	if s.config.AdminToken == "" {
		writeJSON(w, http.StatusServiceUnavailable, map[string]string{"error": "admin token is not configured"})
		return
	}

	var payload adminSessionLoginRequest
	if err := decodeJSONBody(r.Body, &payload); err != nil {
		writeJSON(w, http.StatusBadRequest, map[string]string{"error": err.Error()})
		return
	}
	if payload.Token != s.config.AdminToken {
		writeJSON(w, http.StatusUnauthorized, map[string]string{"error": "invalid admin token"})
		return
	}

	sessionToken, err := s.sessions.Create()
	if err != nil {
		writeJSON(w, http.StatusInternalServerError, map[string]string{"error": "create admin session"})
		return
	}

	http.SetCookie(w, &http.Cookie{
		Name:     adminSessionCookieName,
		Value:    sessionToken,
		Path:     "/",
		HttpOnly: true,
		SameSite: http.SameSiteLaxMode,
		Expires:  time.Now().Add(s.sessions.ttl),
	})
	writeJSON(w, http.StatusOK, map[string]string{"status": "ok"})
}

func (s *AdminHTTPServer) handleLogout(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		writeJSON(w, http.StatusMethodNotAllowed, map[string]string{"error": "method not allowed"})
		return
	}
	s.sessions.Delete(r)
	http.SetCookie(w, &http.Cookie{
		Name:     adminSessionCookieName,
		Value:    "",
		Path:     "/",
		HttpOnly: true,
		Expires:  time.Unix(0, 0),
		MaxAge:   -1,
	})
	writeJSON(w, http.StatusOK, map[string]string{"status": "ok"})
}

func (s *AdminHTTPServer) handleOverview(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		writeJSON(w, http.StatusMethodNotAllowed, map[string]string{"error": "method not allowed"})
		return
	}

	services, signalStatus, _ := s.fetchServiceEnvelopes(r.Context())
	devices := s.management.Registry().List()
	online := onlineDeviceSet(signalStatus)

	recentEvents := []store.AuditEvent{}
	if auditStore := s.management.AuditStore(); auditStore != nil {
		var err error
		recentEvents, err = auditStore.ListAuditEvents(r.Context(), 8)
		if err != nil {
			writeJSON(w, http.StatusInternalServerError, map[string]string{"error": err.Error()})
			return
		}
	}

	response := adminOverviewResponse{
		GeneratedAt: time.Now().UTC(),
		Management: adminManagementSummary{
			Revision:       s.management.Registry().CurrentRevision(),
			DeviceTotal:    len(devices),
			OnlineDevices:  len(online),
			OfflineDevices: max(0, len(devices)-len(online)),
		},
		Services:     services,
		RecentEvents: recentEvents,
	}
	writeJSON(w, http.StatusOK, response)
}

func (s *AdminHTTPServer) handleDevices(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		writeJSON(w, http.StatusMethodNotAllowed, map[string]string{"error": "method not allowed"})
		return
	}

	_, signalStatus, _ := s.fetchServiceEnvelopes(r.Context())
	online := onlineDeviceSet(signalStatus)

	devices := s.management.Registry().List()
	response := make([]adminDeviceResponse, 0, len(devices))
	for _, record := range devices {
		response = append(response, toAdminDevice(record, online[record.ID]))
	}
	writeJSON(w, http.StatusOK, map[string]interface{}{"devices": response})
}

func (s *AdminHTTPServer) handleDeviceByID(w http.ResponseWriter, r *http.Request) {
	deviceID, action := trimDevicePath(r.URL.Path)
	if deviceID == "" {
		writeJSON(w, http.StatusNotFound, map[string]string{"error": "device not found"})
		return
	}

	switch {
	case action == "" && r.Method == http.MethodGet:
		s.handleDeviceDetail(w, r, deviceID)
	case action == "" && r.Method == http.MethodPatch:
		s.handleDevicePatch(w, r, deviceID)
	case action == "disable" && r.Method == http.MethodPost:
		s.handleDeviceDisable(w, r, deviceID, true)
	case action == "enable" && r.Method == http.MethodPost:
		s.handleDeviceDisable(w, r, deviceID, false)
	case action == "" && r.Method == http.MethodDelete:
		s.handleDeviceDelete(w, r, deviceID)
	default:
		writeJSON(w, http.StatusMethodNotAllowed, map[string]string{"error": "method not allowed"})
	}
}

func (s *AdminHTTPServer) handleDeviceDetail(w http.ResponseWriter, r *http.Request, deviceID string) {
	record, ok := s.management.Registry().GetByID(deviceID)
	if !ok {
		writeJSON(w, http.StatusNotFound, map[string]string{"error": "device not found"})
		return
	}

	_, signalStatus, _ := s.fetchServiceEnvelopes(r.Context())
	online := onlineDeviceSet(signalStatus)
	peers := s.management.visiblePeers(deviceID)

	response := adminDeviceDetailResponse{
		Device: toAdminDevice(record, online[deviceID]),
		Peers:  make([]adminPeerResponse, 0, len(peers)),
	}
	for _, peer := range peers {
		response.Peers = append(response.Peers, adminPeerResponse{
			PeerID:     peer.GetPeerId(),
			PublicKey:  peer.GetPublicKey(),
			OverlayIP:  peer.GetOverlay().GetIpv4(),
			AllowedIPs: append([]string(nil), peer.GetAllowedIps()...),
			DirectEndpoint: &adminEndpoint{
				Host: peer.GetDirectEndpoint().GetHost(),
				Port: peer.GetDirectEndpoint().GetPort(),
			},
		})
		if peer.GetDirectEndpoint() == nil {
			response.Peers[len(response.Peers)-1].DirectEndpoint = nil
		}
	}
	writeJSON(w, http.StatusOK, response)
}

func (s *AdminHTTPServer) handleDevicePatch(w http.ResponseWriter, r *http.Request, deviceID string) {
	var payload adminDevicePatchRequest
	if err := decodeJSONBody(r.Body, &payload); err != nil {
		writeJSON(w, http.StatusBadRequest, map[string]string{"error": err.Error()})
		return
	}

	record, ok := s.management.Registry().GetByID(deviceID)
	if !ok {
		writeJSON(w, http.StatusNotFound, map[string]string{"error": "device not found"})
		return
	}

	routes := record.AdvertisedRoutes
	if payload.AdvertisedRoutes != nil {
		normalized, err := normalizeAdvertisedRoutes(payload.AdvertisedRoutes)
		if err != nil {
			writeJSON(w, http.StatusBadRequest, map[string]string{"error": err.Error()})
			return
		}
		if err := validateAdvertisedRoutes(normalized, s.management.overlayNet, s.management.Registry().List(), record.PublicKey); err != nil {
			writeJSON(w, http.StatusBadRequest, map[string]string{"error": err.Error()})
			return
		}
		routes = normalized
	}

	updated, err := s.management.Registry().UpdateMetadata(deviceID, device.MetadataUpdate{
		Name:             payload.Name,
		Labels:           payload.Labels,
		AdvertisedRoutes: routes,
	}, &device.AuditEvent{
		OccurredAt: time.Now().UTC(),
		Actor:      "admin",
		Action:     "admin.device.update",
		DeviceID:   deviceID,
		Summary:    fmt.Sprintf("updated device %s", record.Name),
	})
	if err != nil {
		writeJSON(w, http.StatusInternalServerError, map[string]string{"error": err.Error()})
		return
	}
	writeJSON(w, http.StatusOK, map[string]interface{}{"device": toAdminDevice(updated, false)})
}

func (s *AdminHTTPServer) handleDeviceDisable(w http.ResponseWriter, r *http.Request, deviceID string, disabled bool) {
	action := "enable"
	if disabled {
		action = "disable"
	}

	record, err := s.management.Registry().SetDisabled(deviceID, disabled, &device.AuditEvent{
		OccurredAt: time.Now().UTC(),
		Actor:      "admin",
		Action:     "admin.device." + action,
		DeviceID:   deviceID,
		Summary:    fmt.Sprintf("%sd device %s", action, deviceID),
	})
	if err != nil {
		statusCode := http.StatusInternalServerError
		if strings.Contains(err.Error(), "not found") {
			statusCode = http.StatusNotFound
		}
		writeJSON(w, statusCode, map[string]string{"error": err.Error()})
		return
	}

	writeJSON(w, http.StatusOK, map[string]interface{}{"device": toAdminDevice(record, false)})
}

func (s *AdminHTTPServer) handleDeviceDelete(w http.ResponseWriter, r *http.Request, deviceID string) {
	if err := s.management.Registry().Delete(deviceID, &device.AuditEvent{
		OccurredAt: time.Now().UTC(),
		Actor:      "admin",
		Action:     "admin.device.delete",
		DeviceID:   deviceID,
		Summary:    fmt.Sprintf("deleted device %s", deviceID),
	}); err != nil {
		statusCode := http.StatusInternalServerError
		if strings.Contains(err.Error(), "not found") {
			statusCode = http.StatusNotFound
		}
		writeJSON(w, statusCode, map[string]string{"error": err.Error()})
		return
	}
	writeJSON(w, http.StatusOK, map[string]string{"status": "ok"})
}

func (s *AdminHTTPServer) handleServices(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		writeJSON(w, http.StatusMethodNotAllowed, map[string]string{"error": "method not allowed"})
		return
	}
	services, _, _ := s.fetchServiceEnvelopes(r.Context())
	writeJSON(w, http.StatusOK, map[string]interface{}{"services": services})
}

func (s *AdminHTTPServer) handleConfig(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		writeJSON(w, http.StatusMethodNotAllowed, map[string]string{"error": "method not allowed"})
		return
	}
	writeJSON(w, http.StatusOK, adminConfigSummary{
		GRPCListenAddr:       s.config.GRPCListenAddr,
		HTTPListenAddr:       s.config.HTTPListenAddr,
		OverlayCIDR:          s.config.OverlayCIDR,
		SyncInterval:         s.config.SyncInterval.String(),
		StateDBPath:          s.config.StateDBPath,
		AdminTokenConfigured: s.config.AdminToken != "",
		SignaldStatusURL:     s.config.SignaldStatusURL,
		RelaydStatusURL:      s.config.RelaydStatusURL,
	})
}

func (s *AdminHTTPServer) handleAudit(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		writeJSON(w, http.StatusMethodNotAllowed, map[string]string{"error": "method not allowed"})
		return
	}

	events := []store.AuditEvent{}
	if auditStore := s.management.AuditStore(); auditStore != nil {
		var err error
		events, err = auditStore.ListAuditEvents(r.Context(), 50)
		if err != nil {
			writeJSON(w, http.StatusInternalServerError, map[string]string{"error": err.Error()})
			return
		}
	}
	writeJSON(w, http.StatusOK, map[string]interface{}{"events": events})
}

func (s *AdminHTTPServer) fetchServiceEnvelopes(ctx context.Context) ([]adminServiceEnvelope, observability.SignalStatus, observability.RelayStatus) {
	envelopes := make([]adminServiceEnvelope, 0, 3)
	managementStatus := adminServiceEnvelope{
		Service: "managementd",
		Healthy: true,
		Now:     time.Now().UTC(),
	}
	managementPayload, _ := json.Marshal(map[string]interface{}{
		"revision":     s.management.Registry().CurrentRevision(),
		"device_total": len(s.management.Registry().List()),
	})
	managementStatus.Status = managementPayload
	envelopes = append(envelopes, managementStatus)

	signalEnvelope, signalStatus := s.fetchSignalStatus(ctx)
	envelopes = append(envelopes, signalEnvelope)

	relayEnvelope, relayStatus := s.fetchRelayStatus(ctx)
	envelopes = append(envelopes, relayEnvelope)

	return envelopes, signalStatus, relayStatus
}

func (s *AdminHTTPServer) fetchSignalStatus(ctx context.Context) (adminServiceEnvelope, observability.SignalStatus) {
	envelope := adminServiceEnvelope{
		Service: "signald",
		Healthy: false,
		Now:     time.Now().UTC(),
	}
	if s.config.SignaldStatusURL == "" {
		envelope.Error = "signald status url not configured"
		return envelope, observability.SignalStatus{}
	}

	var raw adminServiceEnvelope
	if err := s.statusClient.GetJSON(ctx, s.config.SignaldStatusURL, &raw); err != nil {
		envelope.Error = err.Error()
		return envelope, observability.SignalStatus{}
	}

	var status observability.SignalStatus
	if err := json.Unmarshal(raw.Status, &status); err != nil {
		envelope.Error = err.Error()
		return envelope, observability.SignalStatus{}
	}
	return raw, status
}

func (s *AdminHTTPServer) fetchRelayStatus(ctx context.Context) (adminServiceEnvelope, observability.RelayStatus) {
	envelope := adminServiceEnvelope{
		Service: "relayd",
		Healthy: false,
		Now:     time.Now().UTC(),
	}
	if s.config.RelaydStatusURL == "" {
		envelope.Error = "relayd status url not configured"
		return envelope, observability.RelayStatus{}
	}

	var raw adminServiceEnvelope
	if err := s.statusClient.GetJSON(ctx, s.config.RelaydStatusURL, &raw); err != nil {
		envelope.Error = err.Error()
		return envelope, observability.RelayStatus{}
	}

	var status observability.RelayStatus
	if err := json.Unmarshal(raw.Status, &status); err != nil {
		envelope.Error = err.Error()
		return envelope, observability.RelayStatus{}
	}
	return raw, status
}

func (m *adminSessionManager) Create() (string, error) {
	raw := make([]byte, 32)
	if _, err := rand.Read(raw); err != nil {
		return "", err
	}
	token := hex.EncodeToString(raw)

	m.mu.Lock()
	defer m.mu.Unlock()
	m.sessions[token] = time.Now().Add(m.ttl)
	return token, nil
}

func (m *adminSessionManager) Validate(r *http.Request) bool {
	cookie, err := r.Cookie(adminSessionCookieName)
	if err != nil || cookie.Value == "" {
		return false
	}

	m.mu.Lock()
	defer m.mu.Unlock()
	expiresAt, ok := m.sessions[cookie.Value]
	if !ok {
		return false
	}
	if time.Now().After(expiresAt) {
		delete(m.sessions, cookie.Value)
		return false
	}
	m.sessions[cookie.Value] = time.Now().Add(m.ttl)
	return true
}

func (m *adminSessionManager) Delete(r *http.Request) {
	cookie, err := r.Cookie(adminSessionCookieName)
	if err != nil {
		return
	}

	m.mu.Lock()
	defer m.mu.Unlock()
	delete(m.sessions, cookie.Value)
}

func trimDevicePath(path string) (deviceID string, action string) {
	trimmed := strings.TrimPrefix(path, "/api/admin/v1/devices/")
	trimmed = strings.Trim(trimmed, "/")
	if trimmed == "" {
		return "", ""
	}
	parts := strings.Split(trimmed, "/")
	if len(parts) == 1 {
		return parts[0], ""
	}
	return parts[0], parts[1]
}

func toAdminDevice(record *device.Record, online bool) adminDeviceResponse {
	response := adminDeviceResponse{
		ID:               record.ID,
		Name:             record.Name,
		PublicKey:        record.PublicKey,
		OS:               record.OS,
		Version:          record.Version,
		OverlayIP:        record.OverlayIP,
		AdvertisedRoutes: append([]string(nil), record.AdvertisedRoutes...),
		Labels:           cloneLabels(record.Labels),
		Disabled:         record.Disabled,
		LastSeen:         record.LastSeen,
		Online:           online,
	}
	if record.DirectEndpoint != nil {
		response.DirectEndpoint = &adminEndpoint{
			Host: record.DirectEndpoint.Host,
			Port: record.DirectEndpoint.Port,
		}
	}
	return response
}

func cloneLabels(labels map[string]string) map[string]string {
	if len(labels) == 0 {
		return map[string]string{}
	}

	cloned := make(map[string]string, len(labels))
	for key, value := range labels {
		cloned[key] = value
	}
	return cloned
}

func decodeJSONBody(body io.ReadCloser, target interface{}) error {
	defer body.Close()
	if err := json.NewDecoder(body).Decode(target); err != nil {
		return fmt.Errorf("decode request body: %w", err)
	}
	return nil
}

func writeJSON(w http.ResponseWriter, statusCode int, payload interface{}) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(statusCode)
	_ = json.NewEncoder(w).Encode(payload)
}

func onlineDeviceSet(signalStatus observability.SignalStatus) map[string]bool {
	online := make(map[string]bool, len(signalStatus.Sessions))
	for _, session := range signalStatus.Sessions {
		online[session.DeviceID] = true
	}
	return online
}

func max(left, right int) int {
	if left > right {
		return left
	}
	return right
}
