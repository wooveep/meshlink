package store

import (
	"context"
	"path/filepath"
	"testing"
	"time"

	"meshlink/server/internal/device"
)

func TestSQLiteStoreRoundTripSnapshotAndAudit(t *testing.T) {
	path := filepath.Join(t.TempDir(), "management.db")
	sqliteStore, err := NewSQLiteStore(path)
	if err != nil {
		t.Fatalf("new sqlite store: %v", err)
	}
	defer sqliteStore.Close()

	now := time.Now().UTC().Truncate(time.Second)
	snapshot := device.Snapshot{
		Revision: 7,
		Records: []*device.Record{
			{
				ID:               "dev-a",
				Name:             "node-a",
				PublicKey:        "pk-a",
				OS:               "linux",
				Version:          "0.1.0",
				OverlayIP:        "100.64.0.1",
				DirectEndpoint:   &device.DirectEndpoint{Host: "192.0.2.10", Port: 51820},
				AdvertisedRoutes: []string{"10.20.0.0/24"},
				Labels:           map[string]string{"site": "lab"},
				Disabled:         true,
				LastSeen:         now,
			},
		},
	}

	if err := sqliteStore.Save(context.Background(), snapshot, &device.AuditEvent{
		OccurredAt: now,
		Actor:      "admin",
		Action:     "admin.device.disable",
		DeviceID:   "dev-a",
		Summary:    "disabled device dev-a",
	}); err != nil {
		t.Fatalf("save sqlite snapshot: %v", err)
	}

	loaded, err := sqliteStore.Load(context.Background())
	if err != nil {
		t.Fatalf("load sqlite snapshot: %v", err)
	}
	if loaded.Revision != 7 {
		t.Fatalf("expected revision 7, got %d", loaded.Revision)
	}
	if len(loaded.Records) != 1 {
		t.Fatalf("expected one record, got %d", len(loaded.Records))
	}
	record := loaded.Records[0]
	if !record.Disabled {
		t.Fatal("expected disabled flag to survive round trip")
	}
	if record.Labels["site"] != "lab" {
		t.Fatalf("expected labels to round trip, got %+v", record.Labels)
	}
	if record.DirectEndpoint == nil || record.DirectEndpoint.Host != "192.0.2.10" {
		t.Fatalf("expected direct endpoint to round trip, got %+v", record.DirectEndpoint)
	}

	events, err := sqliteStore.ListAuditEvents(context.Background(), 10)
	if err != nil {
		t.Fatalf("list audit events: %v", err)
	}
	if len(events) != 1 {
		t.Fatalf("expected one audit event, got %d", len(events))
	}
	if events[0].Action != "admin.device.disable" {
		t.Fatalf("unexpected audit event: %+v", events[0])
	}
}
