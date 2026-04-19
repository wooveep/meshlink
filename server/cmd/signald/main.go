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
	"meshlink/server/internal/signal"
	"meshlink/server/pkg/pb"
)

func main() {
	addr := flag.String("listen", ":10000", "signal gRPC listen address")
	stunAddr := flag.String("stun-listen", ":3479", "STUN UDP listen address")
	statusAddr := flag.String("status-listen", "127.0.0.1:18080", "internal HTTP status listen address")
	managementAddr := flag.String("management-addr", "127.0.0.1:33073", "management gRPC address")
	bootstrapToken := flag.String("bootstrap-token", "meshlink-dev-token", "bootstrap token for signal hello validation")
	heartbeatTimeout := flag.Duration("heartbeat-timeout", 15*time.Second, "signal session heartbeat timeout")
	flag.Parse()

	managementConn, err := grpc.NewClient(
		*managementAddr,
		grpc.WithTransportCredentials(insecure.NewCredentials()),
	)
	if err != nil {
		log.Fatalf("connect management service %s: %v", *managementAddr, err)
	}
	defer managementConn.Close()

	service := signal.NewService(signal.ServiceConfig{
		BootstrapToken:   *bootstrapToken,
		HeartbeatTimeout: *heartbeatTimeout,
		DeviceLookup:     signal.NewManagementDeviceLookup(pb.NewManagementServiceClient(managementConn)),
	})

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	go service.RunCleanup(ctx, time.Second)
	go func() {
		mux := http.NewServeMux()
		mux.HandleFunc("/internal/status/v1/service", func(w http.ResponseWriter, r *http.Request) {
			snapshot := service.StatsSnapshot()
			payload, _ := json.Marshal(observability.SignalStatus{
				ActiveSessions:   snapshot.ActiveSessions,
				Sessions:         toSignalStatuses(snapshot.Sessions),
				RouteHits:        snapshot.RouteHits,
				RouteMisses:      snapshot.RouteMisses,
				ExpiredSessions:  snapshot.ExpiredSessions,
				HeartbeatTimeout: snapshot.HeartbeatTimeout,
			})
			w.Header().Set("Content-Type", "application/json")
			_ = json.NewEncoder(w).Encode(observability.ServiceStatusEnvelope{
				Service: "signald",
				Healthy: true,
				Now:     time.Now().UTC(),
				Status:  json.RawMessage(payload),
			})
		})
		if err := http.ListenAndServe(*statusAddr, mux); err != nil {
			log.Fatalf("serve signal status http: %v", err)
		}
	}()
	go func() {
		if err := signal.ListenAndServeSTUN(ctx, *stunAddr); err != nil {
			log.Fatalf("serve STUN: %v", err)
		}
	}()

	listener, err := net.Listen("tcp", *addr)
	if err != nil {
		log.Fatalf("listen %s: %v", *addr, err)
	}

	server := grpc.NewServer()
	pb.RegisterSignalServiceServer(server, service)

	log.Printf("signald listening on %s", *addr)
	log.Printf("signald STUN listening on %s", *stunAddr)
	log.Printf("signald status HTTP listening on %s", *statusAddr)
	log.Printf("signald using management service %s", *managementAddr)
	if err := server.Serve(listener); err != nil {
		log.Fatalf("serve signal grpc: %v", err)
	}
}

func toSignalStatuses(sessions []signal.SessionStatus) []observability.SignalSessionStatus {
	statuses := make([]observability.SignalSessionStatus, 0, len(sessions))
	for _, session := range sessions {
		statuses = append(statuses, observability.SignalSessionStatus{
			DeviceID: session.DeviceID,
			LastSeen: session.LastSeen,
		})
	}
	return statuses
}
