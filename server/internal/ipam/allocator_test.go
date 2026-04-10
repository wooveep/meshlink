package ipam

import "testing"

func TestAllocatorAllocateStable(t *testing.T) {
	allocator, err := NewAllocator("100.64.0.0/30")
	if err != nil {
		t.Fatalf("new allocator: %v", err)
	}

	first, err := allocator.Allocate("pk-a")
	if err != nil {
		t.Fatalf("allocate first: %v", err)
	}
	second, err := allocator.Allocate("pk-a")
	if err != nil {
		t.Fatalf("allocate second: %v", err)
	}

	if first != second {
		t.Fatalf("expected stable allocation, got %s and %s", first, second)
	}
}

func TestAllocatorAllocateUnique(t *testing.T) {
	allocator, err := NewAllocator("100.64.0.0/29")
	if err != nil {
		t.Fatalf("new allocator: %v", err)
	}

	first, err := allocator.Allocate("pk-a")
	if err != nil {
		t.Fatalf("allocate first: %v", err)
	}
	second, err := allocator.Allocate("pk-b")
	if err != nil {
		t.Fatalf("allocate second: %v", err)
	}

	if first == second {
		t.Fatalf("expected unique addresses, both were %s", first)
	}
}
