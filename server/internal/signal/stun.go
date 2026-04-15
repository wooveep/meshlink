package signal

import (
	"context"
	"encoding/binary"
	"fmt"
	"net"
)

const (
	stunBindingRequest       uint16 = 0x0001
	stunBindingSuccess       uint16 = 0x0101
	stunHeaderLength                = 20
	stunMagicCookie          uint32 = 0x2112A442
	stunAttrXorMappedAddress uint16 = 0x0020
)

func ListenAndServeSTUN(ctx context.Context, addr string) error {
	conn, err := net.ListenPacket("udp", addr)
	if err != nil {
		return fmt.Errorf("listen stun %s: %w", addr, err)
	}
	defer conn.Close()

	go func() {
		<-ctx.Done()
		_ = conn.Close()
	}()

	buffer := make([]byte, 2048)
	for {
		n, peer, err := conn.ReadFrom(buffer)
		if err != nil {
			if ctx.Err() != nil {
				return nil
			}
			return fmt.Errorf("read stun packet: %w", err)
		}

		udpAddr, ok := peer.(*net.UDPAddr)
		if !ok {
			continue
		}

		response, ok := BuildBindingSuccessResponse(buffer[:n], udpAddr)
		if !ok {
			continue
		}

		if _, err := conn.WriteTo(response, peer); err != nil {
			return fmt.Errorf("write stun response: %w", err)
		}
	}
}

func BuildBindingSuccessResponse(packet []byte, addr *net.UDPAddr) ([]byte, bool) {
	transactionID, ok := ParseBindingRequest(packet)
	if !ok || addr == nil {
		return nil, false
	}

	ipv4 := addr.IP.To4()
	if ipv4 == nil {
		return nil, false
	}

	response := make([]byte, stunHeaderLength+12)
	binary.BigEndian.PutUint16(response[0:2], stunBindingSuccess)
	binary.BigEndian.PutUint16(response[2:4], 12)
	binary.BigEndian.PutUint32(response[4:8], stunMagicCookie)
	copy(response[8:20], transactionID[:])

	binary.BigEndian.PutUint16(response[20:22], stunAttrXorMappedAddress)
	binary.BigEndian.PutUint16(response[22:24], 8)
	response[24] = 0
	response[25] = 0x01
	binary.BigEndian.PutUint16(response[26:28], uint16(addr.Port)^uint16(stunMagicCookie>>16))

	cookie := make([]byte, 4)
	binary.BigEndian.PutUint32(cookie, stunMagicCookie)
	for i := 0; i < len(ipv4); i++ {
		response[28+i] = ipv4[i] ^ cookie[i]
	}

	return response, true
}

func ParseBindingRequest(packet []byte) ([12]byte, bool) {
	var transactionID [12]byte
	if len(packet) < stunHeaderLength {
		return transactionID, false
	}
	if binary.BigEndian.Uint16(packet[0:2]) != stunBindingRequest {
		return transactionID, false
	}
	if binary.BigEndian.Uint32(packet[4:8]) != stunMagicCookie {
		return transactionID, false
	}
	copy(transactionID[:], packet[8:20])
	return transactionID, true
}
