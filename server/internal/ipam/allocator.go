package ipam

import (
	"encoding/binary"
	"errors"
	"fmt"
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

func (a *Allocator) Reserve(publicKey, addr string) error {
	a.mu.Lock()
	defer a.mu.Unlock()

	if existing, ok := a.allocations[publicKey]; ok {
		if existing != addr {
			return fmt.Errorf("public key %s already reserved for %s", publicKey, existing)
		}
		return nil
	}

	ip := net.ParseIP(addr).To4()
	if ip == nil {
		return fmt.Errorf("reserve overlay address: invalid IPv4 %s", addr)
	}

	requested := binary.BigEndian.Uint32(ip)
	if requested <= a.base || requested > a.base+a.limit {
		return fmt.Errorf("reserve overlay address: %s outside allocator range", addr)
	}

	for existingKey, existingAddr := range a.allocations {
		if existingKey != publicKey && existingAddr == addr {
			return fmt.Errorf("reserve overlay address: %s already allocated", addr)
		}
	}

	a.allocations[publicKey] = addr
	offset := requested - a.base
	if offset >= a.next {
		a.next = offset + 1
	}
	return nil
}
