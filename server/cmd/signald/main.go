package main

import (
	"flag"
	"log"
)

func main() {
	addr := flag.String("listen", ":10000", "signal gRPC listen address")
	flag.Parse()

	log.Printf("signald scaffold starting on %s", *addr)
	log.Printf("TODO: phase 05 will implement device sessions, candidate routing, and stream fanout")
}
