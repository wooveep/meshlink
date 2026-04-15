package main

import (
	"context"
	"flag"
	"log"
	"net"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"

	"meshlink/server/internal/relay"
	"meshlink/server/internal/signal"
	"meshlink/server/pkg/pb"
)

func main() {
	addr := flag.String("listen", ":3478", "relay control gRPC listen address")
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

	server := grpc.NewServer()
	pb.RegisterRelayServiceServer(server, service)

	log.Printf("relayd listening on %s", *addr)
	log.Printf("relayd advertising UDP relay host %s", resolvedAdvertiseHost)
	log.Printf("relayd using management service %s", *managementAddr)
	if err := server.Serve(listener); err != nil {
		log.Fatalf("serve relay grpc: %v", err)
	}
}
