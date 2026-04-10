package main

import (
	"flag"
	"log"
	"net"
	"time"

	"google.golang.org/grpc"

	"meshlink/server/internal/app"
	"meshlink/server/pkg/pb"
)

func main() {
	addr := flag.String("listen", ":33073", "management gRPC listen address")
	bootstrapToken := flag.String("bootstrap-token", "meshlink-dev-token", "bootstrap token for device registration")
	overlayCIDR := flag.String("overlay-cidr", "100.64.0.0/10", "overlay IPv4 CIDR")
	syncInterval := flag.Duration("sync-interval", 15*time.Second, "config sync heartbeat interval")
	flag.Parse()

	service, err := app.NewManagementService(app.ManagementConfig{
		BootstrapToken: *bootstrapToken,
		OverlayCIDR:    *overlayCIDR,
		SyncInterval:   *syncInterval,
	})
	if err != nil {
		log.Fatalf("init management service: %v", err)
	}

	listener, err := net.Listen("tcp", *addr)
	if err != nil {
		log.Fatalf("listen %s: %v", *addr, err)
	}

	server := grpc.NewServer()
	pb.RegisterManagementServiceServer(server, service)

	log.Printf("managementd listening on %s", *addr)
	log.Printf("overlay network: %s", *overlayCIDR)
	if err := server.Serve(listener); err != nil {
		log.Fatalf("serve: %v", err)
	}
}
