package device

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"sort"
	"sync"
	"time"
)

type Backend interface {
	Load(context.Context) (Snapshot, error)
	Save(context.Context, Snapshot, *AuditEvent) error
}

type Snapshot struct {
	Revision uint64
	Records   []*Record
}

type AuditEvent struct {
	OccurredAt time.Time
	Actor      string
	Action     string
	DeviceID   string
	Summary    string
}

type Registration struct {
	Name             string
	PublicKey        string
	OS               string
	Version          string
	OverlayIP        string
	DirectEndpoint   *DirectEndpoint
	AdvertisedRoutes []string
}

type MetadataUpdate struct {
	Name             *string
	Labels           map[string]string
	AdvertisedRoutes []string
}

type Record struct {
	ID               string
	Name             string
	PublicKey        string
	OS               string
	Version          string
	OverlayIP        string
	DirectEndpoint   *DirectEndpoint
	AdvertisedRoutes []string
	Labels           map[string]string
	Disabled         bool
	LastSeen         time.Time
}

type DirectEndpoint struct {
	Host string
	Port uint32
}

type Registry struct {
	mu          sync.RWMutex
	byID        map[string]*Record
	byKey       map[string]*Record
	revision    uint64
	backend     Backend
	subscribers map[chan string]struct{}
}

func NewRegistry() *Registry {
	return &Registry{
		byID:        make(map[string]*Record),
		byKey:       make(map[string]*Record),
		subscribers: make(map[chan string]struct{}),
	}
}

func NewRegistryWithBackend(ctx context.Context, backend Backend) (*Registry, error) {
	registry := NewRegistry()
	registry.backend = backend

	if backend == nil {
		return registry, nil
	}

	snapshot, err := backend.Load(ctx)
	if err != nil {
		return nil, fmt.Errorf("load registry snapshot: %w", err)
	}
	registry.restoreLocked(snapshot)
	return registry, nil
}

func (r *Registry) Register(input Registration) (*Record, error) {
	now := time.Now().UTC()

	r.mu.Lock()
	defer r.mu.Unlock()

	before := r.snapshotLocked()

	var record *Record
	if existing, ok := r.byKey[input.PublicKey]; ok {
		existing.Name = input.Name
		existing.OS = input.OS
		existing.Version = input.Version
		existing.OverlayIP = input.OverlayIP
		existing.AdvertisedRoutes = cloneStringSlice(input.AdvertisedRoutes)
		existing.LastSeen = now
		if input.DirectEndpoint != nil {
			existing.DirectEndpoint = cloneDirectEndpoint(input.DirectEndpoint)
		}
		record = existing
	} else {
		record = &Record{
			ID:               makeID(input.PublicKey),
			Name:             input.Name,
			PublicKey:        input.PublicKey,
			OS:               input.OS,
			Version:          input.Version,
			OverlayIP:        input.OverlayIP,
			DirectEndpoint:   cloneDirectEndpoint(input.DirectEndpoint),
			AdvertisedRoutes: cloneStringSlice(input.AdvertisedRoutes),
			Labels:           map[string]string{},
			LastSeen:         now,
		}
		r.byKey[input.PublicKey] = record
		r.byID[record.ID] = record
	}

	r.revision++
	if err := r.persistLocked(before, &AuditEvent{
		OccurredAt: now,
		Actor:      "system",
		Action:     "device.register",
		DeviceID:   record.ID,
		Summary:    fmt.Sprintf("registered device %s", record.Name),
	}); err != nil {
		return nil, err
	}

	revision := formatRevision(r.revision)
	subscribers := cloneSubscribers(r.subscribers)
	copy := clone(record)
	r.mu.Unlock()
	notifySubscribers(subscribers, revision)
	r.mu.Lock()
	return copy, nil
}

func (r *Registry) UpdateMetadata(id string, update MetadataUpdate, audit *AuditEvent) (*Record, error) {
	r.mu.Lock()
	defer r.mu.Unlock()

	record, ok := r.byID[id]
	if !ok {
		return nil, fmt.Errorf("device not found")
	}

	before := r.snapshotLocked()
	if update.Name != nil {
		record.Name = *update.Name
	}
	if update.Labels != nil {
		record.Labels = cloneStringMap(update.Labels)
	}
	if update.AdvertisedRoutes != nil {
		record.AdvertisedRoutes = cloneStringSlice(update.AdvertisedRoutes)
	}

	r.revision++
	if err := r.persistLocked(before, audit); err != nil {
		return nil, err
	}

	revision := formatRevision(r.revision)
	subscribers := cloneSubscribers(r.subscribers)
	copy := clone(record)
	r.mu.Unlock()
	notifySubscribers(subscribers, revision)
	r.mu.Lock()
	return copy, nil
}

func (r *Registry) SetDisabled(id string, disabled bool, audit *AuditEvent) (*Record, error) {
	r.mu.Lock()
	defer r.mu.Unlock()

	record, ok := r.byID[id]
	if !ok {
		return nil, fmt.Errorf("device not found")
	}

	before := r.snapshotLocked()
	record.Disabled = disabled

	r.revision++
	if err := r.persistLocked(before, audit); err != nil {
		return nil, err
	}

	revision := formatRevision(r.revision)
	subscribers := cloneSubscribers(r.subscribers)
	copy := clone(record)
	r.mu.Unlock()
	notifySubscribers(subscribers, revision)
	r.mu.Lock()
	return copy, nil
}

func (r *Registry) Delete(id string, audit *AuditEvent) error {
	r.mu.Lock()
	defer r.mu.Unlock()

	record, ok := r.byID[id]
	if !ok {
		return fmt.Errorf("device not found")
	}

	before := r.snapshotLocked()
	delete(r.byID, id)
	delete(r.byKey, record.PublicKey)
	r.revision++

	if err := r.persistLocked(before, audit); err != nil {
		return err
	}

	revision := formatRevision(r.revision)
	subscribers := cloneSubscribers(r.subscribers)
	r.mu.Unlock()
	notifySubscribers(subscribers, revision)
	r.mu.Lock()
	return nil
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

func (r *Registry) snapshotLocked() Snapshot {
	records := make([]*Record, 0, len(r.byID))
	for _, record := range r.byID {
		records = append(records, clone(record))
	}
	sort.Slice(records, func(i, j int) bool {
		return records[i].ID < records[j].ID
	})
	return Snapshot{
		Revision: r.revision,
		Records:   records,
	}
}

func (r *Registry) restoreLocked(snapshot Snapshot) {
	r.byID = make(map[string]*Record, len(snapshot.Records))
	r.byKey = make(map[string]*Record, len(snapshot.Records))
	r.revision = snapshot.Revision
	for _, record := range snapshot.Records {
		copy := clone(record)
		r.byID[copy.ID] = copy
		r.byKey[copy.PublicKey] = copy
	}
}

func (r *Registry) persistLocked(before Snapshot, audit *AuditEvent) error {
	if r.backend == nil {
		return nil
	}

	if err := r.backend.Save(context.Background(), r.snapshotLocked(), audit); err != nil {
		r.restoreLocked(before)
		return fmt.Errorf("persist registry snapshot: %w", err)
	}
	return nil
}

func clone(record *Record) *Record {
	copy := *record
	copy.DirectEndpoint = cloneDirectEndpoint(record.DirectEndpoint)
	copy.AdvertisedRoutes = cloneStringSlice(record.AdvertisedRoutes)
	copy.Labels = cloneStringMap(record.Labels)
	return &copy
}

func cloneDirectEndpoint(endpoint *DirectEndpoint) *DirectEndpoint {
	if endpoint == nil {
		return nil
	}

	copy := *endpoint
	return &copy
}

func cloneSubscribers(subscribers map[chan string]struct{}) []chan string {
	cloned := make([]chan string, 0, len(subscribers))
	for subscriber := range subscribers {
		cloned = append(cloned, subscriber)
	}
	return cloned
}

func cloneStringSlice(values []string) []string {
	if len(values) == 0 {
		return nil
	}

	cloned := make([]string, len(values))
	copy(cloned, values)
	return cloned
}

func cloneStringMap(values map[string]string) map[string]string {
	if len(values) == 0 {
		return map[string]string{}
	}

	cloned := make(map[string]string, len(values))
	for key, value := range values {
		cloned[key] = value
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
