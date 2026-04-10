package main

import (
	"flag"
	"log"
)

func main() {
	addr := flag.String("listen", ":3478", "relay listen address")
	flag.Parse()

	log.Printf("relayd scaffold starting on %s", *addr)
	log.Printf("TODO: phase 06 will implement encrypted packet relay and rate limiting")
}
