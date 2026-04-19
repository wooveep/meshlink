package main

import (
	"flag"
	"log"
	"net"
	"net/http"
	"time"

	"google.golang.org/grpc"

	"meshlink/server/internal/app"
	"meshlink/server/internal/store"
	"meshlink/server/pkg/pb"
)

func main() {
	grpcAddr := flag.String("listen", ":33073", "management gRPC listen address")
	httpAddr := flag.String("http-listen", ":8080", "management admin HTTP listen address")
	bootstrapToken := flag.String("bootstrap-token", "meshlink-dev-token", "bootstrap token for device registration")
	adminToken := flag.String("admin-token", "meshlink-admin-token", "admin token for the web console")
	overlayCIDR := flag.String("overlay-cidr", "100.64.0.0/10", "overlay IPv4 CIDR")
	syncInterval := flag.Duration("sync-interval", 15*time.Second, "config sync heartbeat interval")
	stateDBPath := flag.String("state-db", "var/lib/meshlink/management.db", "sqlite database path for management state")
	signaldStatusURL := flag.String("signald-status-url", "http://127.0.0.1:18080/internal/status/v1/service", "internal signald status URL")
	relaydStatusURL := flag.String("relayd-status-url", "http://127.0.0.1:18081/internal/status/v1/service", "internal relayd status URL")
	flag.Parse()

	stateStore, err := store.NewSQLiteStore(*stateDBPath)
	if err != nil {
		log.Fatalf("init sqlite state store: %v", err)
	}
	defer stateStore.Close()

	service, err := app.NewManagementService(app.ManagementConfig{
		BootstrapToken: *bootstrapToken,
		OverlayCIDR:    *overlayCIDR,
		SyncInterval:   *syncInterval,
		StateStore:     stateStore,
		AuditStore:     stateStore,
	})
	if err != nil {
		log.Fatalf("init management service: %v", err)
	}

	adminServer := app.NewAdminHTTPServer(service, app.AdminHTTPConfig{
		AdminToken:       *adminToken,
		GRPCListenAddr:   *grpcAddr,
		HTTPListenAddr:   *httpAddr,
		OverlayCIDR:      *overlayCIDR,
		SyncInterval:     *syncInterval,
		StateDBPath:      *stateDBPath,
		SignaldStatusURL: *signaldStatusURL,
		RelaydStatusURL:  *relaydStatusURL,
	})

	grpcListener, err := net.Listen("tcp", *grpcAddr)
	if err != nil {
		log.Fatalf("listen %s: %v", *grpcAddr, err)
	}

	grpcServer := grpc.NewServer()
	pb.RegisterManagementServiceServer(grpcServer, service)

	go func() {
		log.Printf("managementd admin HTTP listening on %s", *httpAddr)
		if err := http.ListenAndServe(*httpAddr, adminServer.Handler()); err != nil {
			log.Fatalf("serve admin http: %v", err)
		}
	}()

	log.Printf("managementd gRPC listening on %s", *grpcAddr)
	log.Printf("overlay network: %s", *overlayCIDR)
	log.Printf("managementd state database: %s", *stateDBPath)
	if err := grpcServer.Serve(grpcListener); err != nil {
		log.Fatalf("serve: %v", err)
	}
}
