package store

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"time"

	_ "modernc.org/sqlite"

	"meshlink/server/internal/device"
)

type SQLiteStore struct {
	db *sql.DB
}

type AuditEvent struct {
	ID         int64     `json:"id"`
	OccurredAt time.Time `json:"occurred_at"`
	Actor      string    `json:"actor"`
	Action     string    `json:"action"`
	DeviceID   string    `json:"device_id,omitempty"`
	Summary    string    `json:"summary"`
}

func NewSQLiteStore(path string) (*SQLiteStore, error) {
	if path == "" {
		return nil, fmt.Errorf("sqlite path is required")
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return nil, fmt.Errorf("create sqlite parent directory: %w", err)
	}

	db, err := sql.Open("sqlite", path)
	if err != nil {
		return nil, fmt.Errorf("open sqlite database: %w", err)
	}

	store := &SQLiteStore{db: db}
	if err := store.init(context.Background()); err != nil {
		_ = db.Close()
		return nil, err
	}

	return store, nil
}

func (s *SQLiteStore) Close() error {
	if s == nil || s.db == nil {
		return nil
	}
	return s.db.Close()
}

func (s *SQLiteStore) Load(ctx context.Context) (device.Snapshot, error) {
	rows, err := s.db.QueryContext(ctx, `
		SELECT id, name, public_key, os, version, overlay_ip, direct_host, direct_port,
		       advertised_routes_json, labels_json, disabled, last_seen
		FROM devices
		ORDER BY id
	`)
	if err != nil {
		return device.Snapshot{}, fmt.Errorf("query devices: %w", err)
	}
	defer rows.Close()

	records := make([]*device.Record, 0)
	for rows.Next() {
		var (
			record              device.Record
			directHost          sql.NullString
			directPort          sql.NullInt64
			advertisedRoutesRaw string
			labelsRaw           string
			disabled            bool
			lastSeenRaw         string
		)
		if err := rows.Scan(
			&record.ID,
			&record.Name,
			&record.PublicKey,
			&record.OS,
			&record.Version,
			&record.OverlayIP,
			&directHost,
			&directPort,
			&advertisedRoutesRaw,
			&labelsRaw,
			&disabled,
			&lastSeenRaw,
		); err != nil {
			return device.Snapshot{}, fmt.Errorf("scan device row: %w", err)
		}

		if directHost.Valid && directPort.Valid {
			record.DirectEndpoint = &device.DirectEndpoint{
				Host: directHost.String,
				Port: uint32(directPort.Int64),
			}
		}
		if err := json.Unmarshal([]byte(advertisedRoutesRaw), &record.AdvertisedRoutes); err != nil {
			return device.Snapshot{}, fmt.Errorf("decode advertised routes: %w", err)
		}
		if err := json.Unmarshal([]byte(labelsRaw), &record.Labels); err != nil {
			return device.Snapshot{}, fmt.Errorf("decode labels: %w", err)
		}
		record.Disabled = disabled
		if lastSeenRaw != "" {
			lastSeen, err := time.Parse(time.RFC3339Nano, lastSeenRaw)
			if err != nil {
				return device.Snapshot{}, fmt.Errorf("parse last seen: %w", err)
			}
			record.LastSeen = lastSeen
		}
		records = append(records, &record)
	}
	if err := rows.Err(); err != nil {
		return device.Snapshot{}, fmt.Errorf("iterate device rows: %w", err)
	}

	var revision uint64
	if err := s.db.QueryRowContext(ctx, `SELECT COALESCE(value, '0') FROM metadata WHERE key = 'revision'`).Scan(&revision); err != nil {
		if !errors.Is(err, sql.ErrNoRows) {
			return device.Snapshot{}, fmt.Errorf("query revision: %w", err)
		}
	}

	return device.Snapshot{
		Revision: revision,
		Records:  records,
	}, nil
}

func (s *SQLiteStore) Save(ctx context.Context, snapshot device.Snapshot, audit *device.AuditEvent) error {
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return fmt.Errorf("begin sqlite transaction: %w", err)
	}
	defer func() {
		if err != nil {
			_ = tx.Rollback()
		}
	}()

	if _, err = tx.ExecContext(ctx, `DELETE FROM devices`); err != nil {
		return fmt.Errorf("truncate devices: %w", err)
	}

	stmt, err := tx.PrepareContext(ctx, `
		INSERT INTO devices (
			id, name, public_key, os, version, overlay_ip, direct_host, direct_port,
			advertised_routes_json, labels_json, disabled, last_seen
		) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
	`)
	if err != nil {
		return fmt.Errorf("prepare device insert: %w", err)
	}
	defer stmt.Close()

	for _, record := range snapshot.Records {
		advertisedRoutesRaw, err := json.Marshal(record.AdvertisedRoutes)
		if err != nil {
			return fmt.Errorf("encode advertised routes: %w", err)
		}
		labelsRaw, err := json.Marshal(record.Labels)
		if err != nil {
			return fmt.Errorf("encode labels: %w", err)
		}

		var (
			directHost any
			directPort any
		)
		if record.DirectEndpoint != nil {
			directHost = record.DirectEndpoint.Host
			directPort = int64(record.DirectEndpoint.Port)
		}

		if _, err = stmt.ExecContext(
			ctx,
			record.ID,
			record.Name,
			record.PublicKey,
			record.OS,
			record.Version,
			record.OverlayIP,
			directHost,
			directPort,
			string(advertisedRoutesRaw),
			string(labelsRaw),
			record.Disabled,
			record.LastSeen.UTC().Format(time.RFC3339Nano),
		); err != nil {
			return fmt.Errorf("insert device %s: %w", record.ID, err)
		}
	}

	if _, err = tx.ExecContext(
		ctx,
		`INSERT INTO metadata(key, value) VALUES('revision', ?)
		 ON CONFLICT(key) DO UPDATE SET value = excluded.value`,
		snapshot.Revision,
	); err != nil {
		return fmt.Errorf("upsert revision metadata: %w", err)
	}

	if audit != nil {
		if _, err = tx.ExecContext(
			ctx,
			`INSERT INTO audit_events (occurred_at, actor, action, device_id, summary) VALUES (?, ?, ?, ?, ?)`,
			audit.OccurredAt.UTC().Format(time.RFC3339Nano),
			audit.Actor,
			audit.Action,
			audit.DeviceID,
			audit.Summary,
		); err != nil {
			return fmt.Errorf("insert audit event: %w", err)
		}
	}

	if err = tx.Commit(); err != nil {
		return fmt.Errorf("commit sqlite transaction: %w", err)
	}
	return nil
}

func (s *SQLiteStore) ListAuditEvents(ctx context.Context, limit int) ([]AuditEvent, error) {
	if limit <= 0 {
		limit = 20
	}

	rows, err := s.db.QueryContext(
		ctx,
		`SELECT id, occurred_at, actor, action, device_id, summary
		 FROM audit_events
		 ORDER BY id DESC
		 LIMIT ?`,
		limit,
	)
	if err != nil {
		return nil, fmt.Errorf("query audit events: %w", err)
	}
	defer rows.Close()

	events := make([]AuditEvent, 0, limit)
	for rows.Next() {
		var (
			event         AuditEvent
			occurredAtRaw string
		)
		if err := rows.Scan(&event.ID, &occurredAtRaw, &event.Actor, &event.Action, &event.DeviceID, &event.Summary); err != nil {
			return nil, fmt.Errorf("scan audit event: %w", err)
		}
		event.OccurredAt, err = time.Parse(time.RFC3339Nano, occurredAtRaw)
		if err != nil {
			return nil, fmt.Errorf("parse audit event timestamp: %w", err)
		}
		events = append(events, event)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate audit events: %w", err)
	}
	return events, nil
}

func (s *SQLiteStore) init(ctx context.Context) error {
	statements := []string{
		`CREATE TABLE IF NOT EXISTS metadata (
			key TEXT PRIMARY KEY,
			value TEXT NOT NULL
		)`,
		`CREATE TABLE IF NOT EXISTS devices (
			id TEXT PRIMARY KEY,
			name TEXT NOT NULL,
			public_key TEXT NOT NULL UNIQUE,
			os TEXT NOT NULL,
			version TEXT NOT NULL,
			overlay_ip TEXT NOT NULL,
			direct_host TEXT,
			direct_port INTEGER,
			advertised_routes_json TEXT NOT NULL,
			labels_json TEXT NOT NULL,
			disabled INTEGER NOT NULL DEFAULT 0,
			last_seen TEXT NOT NULL
		)`,
		`CREATE TABLE IF NOT EXISTS audit_events (
			id INTEGER PRIMARY KEY AUTOINCREMENT,
			occurred_at TEXT NOT NULL,
			actor TEXT NOT NULL,
			action TEXT NOT NULL,
			device_id TEXT NOT NULL DEFAULT '',
			summary TEXT NOT NULL
		)`,
	}

	for _, statement := range statements {
		if _, err := s.db.ExecContext(ctx, statement); err != nil {
			return fmt.Errorf("init sqlite schema: %w", err)
		}
	}
	return nil
}
