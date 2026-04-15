package signal

import (
	"net"
	"testing"
)

func TestBuildBindingSuccessResponse(t *testing.T) {
	request := []byte{
		0x00, 0x01, 0x00, 0x00,
		0x21, 0x12, 0xA4, 0x42,
		0x01, 0x02, 0x03, 0x04,
		0x05, 0x06, 0x07, 0x08,
		0x09, 0x0A, 0x0B, 0x0C,
	}

	response, ok := BuildBindingSuccessResponse(request, &net.UDPAddr{
		IP:   net.IPv4(198, 51, 100, 10),
		Port: 54321,
	})
	if !ok {
		t.Fatal("expected binding request to produce a response")
	}
	if len(response) != 32 {
		t.Fatalf("unexpected response length: %d", len(response))
	}
	if response[0] != 0x01 || response[1] != 0x01 {
		t.Fatalf("unexpected response type: %x %x", response[0], response[1])
	}
}
