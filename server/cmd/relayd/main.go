package main

import (
	"context"
	"encoding/json"
	"flag"
	"log"
	"net"
	"net/http"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"

	"meshlink/server/internal/observability"
	"meshlink/server/internal/relay"
	"meshlink/server/internal/signal"
	"meshlink/server/pkg/pb"
)

func main() {
	addr := flag.String("listen", ":3478", "relay control gRPC listen address")
	statusAddr := flag.String("status-listen", "127.0.0.1:18081", "internal HTTP status listen address")
	managementAddr := flag.String("management-addr", "127.0.0.1:33073", "management gRPC address")
	bootstrapToken := flag.String("bootstrap-token", "meshlink-dev-token", "bootstrap token for relay reservation validation")
	sessionTTL := flag.Duration("session-ttl", 30*time.Second, "relay session reservation TTL")
	cleanupInterval := flag.Duration("cleanup-interval", time.Second, "relay session cleanup interval")
	advertiseHost := flag.String("advertise-host", "", "host returned to clients for relay UDP sessions")
	flag.Parse()

	listener, err := net.Listen("tcp", *addr)
	if err != nil {
		log.Fatalf("listen %s: %v", *addr, err)
	}

	resolvedAdvertiseHost := relay.ResolveAdvertiseHost(*addr, *advertiseHost)

	managementConn, err := grpc.NewClient(
		*managementAddr,
		grpc.WithTransportCredentials(insecure.NewCredentials()),
	)
	if err != nil {
		log.Fatalf("connect management service %s: %v", *managementAddr, err)
	}
	defer managementConn.Close()

	service, err := relay.NewService(relay.ServiceConfig{
		BootstrapToken: *bootstrapToken,
		SessionTTL:     *sessionTTL,
		AdvertiseHost:  resolvedAdvertiseHost,
		DeviceLookup:   signal.NewManagementDeviceLookup(pb.NewManagementServiceClient(managementConn)),
	})
	if err != nil {
		log.Fatalf("init relay service: %v", err)
	}

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go service.RunCleanup(ctx, *cleanupInterval)
	go func() {
		mux := http.NewServeMux()
		mux.HandleFunc("/internal/status/v1/service", func(w http.ResponseWriter, r *http.Request) {
			snapshot := service.StatsSnapshot()
			payload, _ := json.Marshal(observability.RelayStatus{
				ActiveSessions: snapshot.ActiveSessions,
				Sessions:       toRelayStatuses(snapshot.Sessions),
				Reservations:   snapshot.Reservations,
				ReapedSessions: snapshot.ReapedSessions,
				SessionTTL:     snapshot.SessionTTL,
				AdvertiseHost:  snapshot.AdvertiseHost,
			})
			w.Header().Set("Content-Type", "application/json")
			_ = json.NewEncoder(w).Encode(observability.ServiceStatusEnvelope{
				Service: "relayd",
				Healthy: true,
				Now:     time.Now().UTC(),
				Status:  json.RawMessage(payload),
			})
		})
		if err := http.ListenAndServe(*statusAddr, mux); err != nil {
			log.Fatalf("serve relay status http: %v", err)
		}
	}()

	server := grpc.NewServer()
	pb.RegisterRelayServiceServer(server, service)

	log.Printf("relayd listening on %s", *addr)
	log.Printf("relayd status HTTP listening on %s", *statusAddr)
	log.Printf("relayd advertising UDP relay host %s", resolvedAdvertiseHost)
	log.Printf("relayd using management service %s", *managementAddr)
	if err := server.Serve(listener); err != nil {
		log.Fatalf("serve relay grpc: %v", err)
	}
}

func toRelayStatuses(sessions []relay.SessionStatus) []observability.RelaySessionStatus {
	statuses := make([]observability.RelaySessionStatus, 0, len(sessions))
	for _, session := range sessions {
		statuses = append(statuses, observability.RelaySessionStatus{
			SessionID: session.SessionID,
			Port:      session.Port,
			Members:   append([]string(nil), session.Members...),
			ExpiresAt: session.ExpiresAt,
		})
	}
	return statuses
}
