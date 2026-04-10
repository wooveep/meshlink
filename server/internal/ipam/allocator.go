package ipam

import (
	"encoding/binary"
	"errors"
	"net"
	"sync"
)

var ErrAddressPoolExhausted = errors.New("overlay address pool exhausted")

type Allocator struct {
	mu          sync.Mutex
	base        uint32
	limit       uint32
	next        uint32
	allocations map[string]string
}

func NewAllocator(cidr string) (*Allocator, error) {
	ip, network, err := net.ParseCIDR(cidr)
	if err != nil {
		return nil, err
	}

	ip4 := ip.To4()
	if ip4 == nil {
		return nil, errors.New("only IPv4 overlay pools are currently supported")
	}

	maskSize, bits := network.Mask.Size()
	hosts := uint32(1) << uint32(bits-maskSize)
	if hosts <= 2 {
		return nil, errors.New("overlay pool too small")
	}

	base := binary.BigEndian.Uint32(ip4)
	return &Allocator{
		base:        base,
		limit:       hosts - 2,
		next:        1,
		allocations: make(map[string]string),
	}, nil
}

func (a *Allocator) Allocate(publicKey string) (string, error) {
	a.mu.Lock()
	defer a.mu.Unlock()

	if addr, ok := a.allocations[publicKey]; ok {
		return addr, nil
	}

	if a.next > a.limit {
		return "", ErrAddressPoolExhausted
	}

	current := a.base + a.next
	a.next++

	buf := make([]byte, 4)
	binary.BigEndian.PutUint32(buf, current)
	addr := net.IP(buf).String()
	a.allocations[publicKey] = addr
	return addr, nil
}
