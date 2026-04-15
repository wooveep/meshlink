package main

import (
	"context"
	"flag"
	"log"
	"net"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"

	"meshlink/server/internal/signal"
	"meshlink/server/pkg/pb"
)

func main() {
	addr := flag.String("listen", ":10000", "signal gRPC listen address")
	stunAddr := flag.String("stun-listen", ":3479", "STUN UDP listen address")
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
	log.Printf("signald using management service %s", *managementAddr)
	if err := server.Serve(listener); err != nil {
		log.Fatalf("serve signal grpc: %v", err)
	}
}
