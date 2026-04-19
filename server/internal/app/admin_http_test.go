package app

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/cookiejar"
	"net/http/httptest"
	"path/filepath"
	"testing"
	"time"

	"meshlink/server/internal/store"
	"meshlink/server/pkg/pb"
)

func TestAdminHTTPRequiresLoginAndCanUpdateDevice(t *testing.T) {
	service, cleanup := newAdminTestManagementService(t)
	defer cleanup()

	registerResponse, err := service.RegisterDevice(context.Background(), &pb.RegisterDeviceRequest{
		Name:      "client-a",
		PublicKey: "pk-a",
		Token:     "meshlink-dev-token",
		Os:        "linux",
		Version:   "0.1.0",
	})
	if err != nil {
		t.Fatalf("register device: %v", err)
	}

	signaldStatus := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		writeJSON(w, http.StatusOK, map[string]interface{}{
			"service": "signald",
			"healthy": true,
			"now":     time.Now().UTC(),
			"status": map[string]interface{}{
				"active_sessions": 1,
				"sessions": []map[string]interface{}{
					{
						"device_id": registerResponse.GetDevice().GetId(),
						"last_seen": time.Now().UTC(),
					},
				},
				"route_hits":        3,
				"route_misses":      1,
				"expired_sessions":  0,
				"heartbeat_timeout": "15s",
			},
		})
	}))
	defer signaldStatus.Close()

	relaydStatus := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		writeJSON(w, http.StatusOK, map[string]interface{}{
			"service": "relayd",
			"healthy": true,
			"now":     time.Now().UTC(),
			"status": map[string]interface{}{
				"active_sessions": 0,
				"sessions":        []map[string]interface{}{},
				"reservations":    0,
				"reaped_sessions": 0,
				"session_ttl":     "30s",
				"advertise_host":  "127.0.0.1",
			},
		})
	}))
	defer relaydStatus.Close()

	adminServer := httptest.NewServer(NewAdminHTTPServer(service, AdminHTTPConfig{
		AdminToken:       "meshlink-admin-token",
		GRPCListenAddr:   ":33073",
		HTTPListenAddr:   ":8080",
		OverlayCIDR:      "100.64.0.0/24",
		SyncInterval:     15 * time.Second,
		StateDBPath:      "test.db",
		SignaldStatusURL: signaldStatus.URL,
		RelaydStatusURL:  relaydStatus.URL,
	}).Handler())
	defer adminServer.Close()

	client := adminHTTPClient(t)

	response, err := client.Get(adminServer.URL + "/api/admin/v1/devices")
	if err != nil {
		t.Fatalf("request devices without auth: %v", err)
	}
	if response.StatusCode != http.StatusUnauthorized {
		t.Fatalf("expected unauthorized before login, got %d", response.StatusCode)
	}
	response.Body.Close()

	mustAdminRequest(t, client, http.MethodPost, adminServer.URL+"/api/admin/v1/session/login", map[string]string{
		"token": "meshlink-admin-token",
	})

	body := mustAdminRequest(t, client, http.MethodPatch, adminServer.URL+"/api/admin/v1/devices/"+registerResponse.GetDevice().GetId(), map[string]interface{}{
		"name":              "client-a-renamed",
		"labels":            map[string]string{"site": "lab"},
		"advertised_routes": []string{"10.20.0.0/24"},
	})

	var patchResponse struct {
		Device Device `json:"device"`
	}
	if err := json.Unmarshal(body, &patchResponse); err != nil {
		t.Fatalf("unmarshal patch response: %v", err)
	}
	if patchResponse.Device.Name != "client-a-renamed" {
		t.Fatalf("expected updated name, got %+v", patchResponse.Device)
	}

	body = mustAdminRequest(t, client, http.MethodPost, adminServer.URL+"/api/admin/v1/devices/"+registerResponse.GetDevice().GetId()+"/disable", map[string]interface{}{})
	if len(body) == 0 {
		t.Fatal("expected disable response body")
	}

	record, ok := service.Registry().GetByID(registerResponse.GetDevice().GetId())
	if !ok {
		t.Fatalf("expected updated device %s", registerResponse.GetDevice().GetId())
	}
	if !record.Disabled {
		t.Fatalf("expected device to be disabled, got %+v", record)
	}
	if record.Labels["site"] != "lab" {
		t.Fatalf("expected labels to be updated, got %+v", record.Labels)
	}
}

func newAdminTestManagementService(t *testing.T) (*ManagementService, func()) {
	t.Helper()

	sqliteStore, err := store.NewSQLiteStore(filepath.Join(t.TempDir(), "management.db"))
	if err != nil {
		t.Fatalf("new sqlite store: %v", err)
	}

	service, err := NewManagementService(ManagementConfig{
		BootstrapToken: "meshlink-dev-token",
		OverlayCIDR:    "100.64.0.0/24",
		SyncInterval:   time.Hour,
		StateStore:     sqliteStore,
		AuditStore:     sqliteStore,
	})
	if err != nil {
		t.Fatalf("new management service: %v", err)
	}
	return service, func() { _ = sqliteStore.Close() }
}

func adminHTTPClient(t *testing.T) *http.Client {
	t.Helper()

	jar, err := cookiejar.New(nil)
	if err != nil {
		t.Fatalf("new cookie jar: %v", err)
	}
	return &http.Client{Jar: jar}
}

func mustAdminRequest(t *testing.T, client *http.Client, method, url string, payload interface{}) []byte {
	t.Helper()

	var body bytes.Buffer
	if payload != nil {
		if err := json.NewEncoder(&body).Encode(payload); err != nil {
			t.Fatalf("encode admin payload: %v", err)
		}
	}

	req, err := http.NewRequest(method, url, &body)
	if err != nil {
		t.Fatalf("new admin request: %v", err)
	}
	if payload != nil {
		req.Header.Set("Content-Type", "application/json")
	}

	response, err := client.Do(req)
	if err != nil {
		t.Fatalf("do admin request: %v", err)
	}
	defer response.Body.Close()
	if response.StatusCode >= 300 {
		t.Fatalf("unexpected admin response %d", response.StatusCode)
	}

	data := new(bytes.Buffer)
	if _, err := data.ReadFrom(response.Body); err != nil {
		t.Fatalf("read admin response: %v", err)
	}
	return data.Bytes()
}

type Device = adminDeviceResponse
