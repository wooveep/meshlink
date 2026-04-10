package device

import (
	"testing"
	"time"
)

func TestRegistryRegisterIdempotentByPublicKey(t *testing.T) {
	registry := NewRegistry()

	first := registry.Register(Registration{
		Name:      "node-a",
		PublicKey: "pk-a",
		OS:        "linux",
		Version:   "0.1.0",
		OverlayIP: "100.64.0.1",
	})
	second := registry.Register(Registration{
		Name:      "node-a-new",
		PublicKey: "pk-a",
		OS:        "linux",
		Version:   "0.1.1",
		OverlayIP: "100.64.0.1",
	})

	if first.ID != second.ID {
		t.Fatalf("expected same device id, got %s and %s", first.ID, second.ID)
	}
	if second.Name != "node-a-new" {
		t.Fatalf("expected latest metadata to win, got %s", second.Name)
	}
	if registry.CurrentRevision() != "00000000000000000002" {
		t.Fatalf("expected revision to advance on each registration, got %s", registry.CurrentRevision())
	}
}

func TestRegistryPreservesDirectEndpointWhenOmitted(t *testing.T) {
	registry := NewRegistry()

	registry.Register(Registration{
		Name:      "node-a",
		PublicKey: "pk-a",
		OverlayIP: "100.64.0.1",
		DirectEndpoint: &DirectEndpoint{
			Host: "198.51.100.10",
			Port: 51820,
		},
	})

	record := registry.Register(Registration{
		Name:      "node-a-new",
		PublicKey: "pk-a",
		OverlayIP: "100.64.0.1",
	})

	if record.DirectEndpoint == nil {
		t.Fatal("expected direct endpoint to be preserved")
	}
	if record.DirectEndpoint.Host != "198.51.100.10" || record.DirectEndpoint.Port != 51820 {
		t.Fatalf("unexpected direct endpoint: %+v", record.DirectEndpoint)
	}
}

func TestRegistrySubscribeNotifiesOnChanges(t *testing.T) {
	registry := NewRegistry()
	updates, cancel := registry.Subscribe()
	defer cancel()

	registry.Register(Registration{
		Name:      "node-a",
		PublicKey: "pk-a",
		OS:        "linux",
		Version:   "0.1.0",
		OverlayIP: "100.64.0.1",
	})

	select {
	case revision := <-updates:
		if revision != "00000000000000000001" {
			t.Fatalf("expected revision 1, got %s", revision)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("expected registry update notification")
	}
}

func TestRegistryListReturnsSortedCopies(t *testing.T) {
	registry := NewRegistry()
	registry.Register(Registration{Name: "node-b", PublicKey: "pk-b", OverlayIP: "100.64.0.2"})
	registry.Register(Registration{Name: "node-a", PublicKey: "pk-a", OverlayIP: "100.64.0.1"})

	records := registry.List()
	if len(records) != 2 {
		t.Fatalf("expected 2 records, got %d", len(records))
	}
	if records[0].ID > records[1].ID {
		t.Fatalf("expected sorted record ids, got %s then %s", records[0].ID, records[1].ID)
	}

	records[0].Name = "mutated"
	again, ok := registry.GetByID(records[0].ID)
	if !ok {
		t.Fatalf("expected to fetch record %s", records[0].ID)
	}
	if again.Name == "mutated" {
		t.Fatal("expected list to return copies")
	}
}
