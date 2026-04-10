package device

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"sort"
	"sync"
)

type Registration struct {
	Name      string
	PublicKey string
	OS        string
	Version   string
	OverlayIP string
}

type Record struct {
	ID        string
	Name      string
	PublicKey string
	OS        string
	Version   string
	OverlayIP string
}

type Registry struct {
	mu          sync.RWMutex
	byID        map[string]*Record
	byKey       map[string]*Record
	revision    uint64
	subscribers map[chan string]struct{}
}

func NewRegistry() *Registry {
	return &Registry{
		byID:        make(map[string]*Record),
		byKey:       make(map[string]*Record),
		subscribers: make(map[chan string]struct{}),
	}
}

func (r *Registry) Register(input Registration) *Record {
	r.mu.Lock()

	if existing, ok := r.byKey[input.PublicKey]; ok {
		existing.Name = input.Name
		existing.OS = input.OS
		existing.Version = input.Version
		existing.OverlayIP = input.OverlayIP
		r.revision++
		record := clone(existing)
		revision := formatRevision(r.revision)
		subscribers := cloneSubscribers(r.subscribers)
		r.mu.Unlock()
		notifySubscribers(subscribers, revision)
		return record
	}

	record := &Record{
		ID:        makeID(input.PublicKey),
		Name:      input.Name,
		PublicKey: input.PublicKey,
		OS:        input.OS,
		Version:   input.Version,
		OverlayIP: input.OverlayIP,
	}
	r.byKey[input.PublicKey] = record
	r.byID[record.ID] = record
	r.revision++
	copy := clone(record)
	revision := formatRevision(r.revision)
	subscribers := cloneSubscribers(r.subscribers)
	r.mu.Unlock()
	notifySubscribers(subscribers, revision)
	return copy
}

func (r *Registry) GetByID(id string) (*Record, bool) {
	r.mu.RLock()
	defer r.mu.RUnlock()

	record, ok := r.byID[id]
	if !ok {
		return nil, false
	}
	return clone(record), true
}

func (r *Registry) List() []*Record {
	r.mu.RLock()
	defer r.mu.RUnlock()

	records := make([]*Record, 0, len(r.byID))
	for _, record := range r.byID {
		records = append(records, clone(record))
	}
	sort.Slice(records, func(i, j int) bool {
		return records[i].ID < records[j].ID
	})
	return records
}

func (r *Registry) CurrentRevision() string {
	r.mu.RLock()
	defer r.mu.RUnlock()
	return formatRevision(r.revision)
}

func (r *Registry) Subscribe() (<-chan string, func()) {
	ch := make(chan string, 1)

	r.mu.Lock()
	r.subscribers[ch] = struct{}{}
	r.mu.Unlock()

	cancel := func() {
		r.mu.Lock()
		delete(r.subscribers, ch)
		r.mu.Unlock()
	}

	return ch, cancel
}

func clone(record *Record) *Record {
	copy := *record
	return &copy
}

func cloneSubscribers(subscribers map[chan string]struct{}) []chan string {
	cloned := make([]chan string, 0, len(subscribers))
	for subscriber := range subscribers {
		cloned = append(cloned, subscriber)
	}
	return cloned
}

func notifySubscribers(subscribers []chan string, revision string) {
	for _, subscriber := range subscribers {
		select {
		case subscriber <- revision:
		default:
			select {
			case <-subscriber:
			default:
			}
			select {
			case subscriber <- revision:
			default:
			}
		}
	}
}

func formatRevision(revision uint64) string {
	return fmt.Sprintf("%020d", revision)
}

func makeID(publicKey string) string {
	sum := sha256.Sum256([]byte(publicKey))
	return "dev-" + hex.EncodeToString(sum[:6])
}
