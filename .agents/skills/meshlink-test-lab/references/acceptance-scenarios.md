# Acceptance Scenarios

## Phase 01

1. Start `managementd`
2. Start one client
3. Verify registration succeeded
4. Verify config stream is received

## Phase 02

1. Start `managementd`
2. Start two clients
3. Verify each client receives the other's peer record

## Phase 03

1. Start two Linux clients with static reachable endpoints
2. Verify WireGuard tunnel is configured
3. Verify overlay ping works
