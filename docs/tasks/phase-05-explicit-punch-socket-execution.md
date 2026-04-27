# Phase 05 Explicit Punch Socket Execution Plan

## Goal

Turn the current Phase 05 dual-NAT root-cause analysis into a concrete,
commit-sized implementation plan that closes the acceptance gap without mixing
it with a larger Linux data-plane ownership rewrite.

## Implementation Status

Status: `completed`

The current repo state now lands this plan as one integrated Phase 05 slice:

1. `PunchResult` carries an explicit `observed_candidate`
2. `agent-core` owns a long-lived `PunchSocket` runtime for STUN and runtime state
3. public STUN candidates are sourced as `punch-socket-stun` only when the
   punch socket happens to be port-aligned
4. peer-observed source candidates are now derived from WireGuard runtime
   telemetry rather than userspace same-port probe receive
5. late remote failure results no longer override a locally observed direct success
6. Linux now closes dual-NAT with an explicit same-port public UDP proxy in
   front of kernel WireGuard, so userspace owns the public socket while kernel
   WireGuard still owns crypto and tunnel state
7. `remote_direct_progress` no longer misuses
   `PunchResult.selected_candidate` as a remote endpoint override, which was
   the last post-handshake proxy-mapping bug

Final closure details:

1. the public proxy owns `0.0.0.0:<listen_port>` and exposes per-peer loopback
   ingress sockets for kernel WireGuard peers
2. WireGuard listens on an internally allocated loopback port instead of the
   public WAN port
3. outbound encrypted packets flow loopback ingress -> public proxy socket ->
   remote endpoint
4. inbound public UDP flows public proxy socket -> per-peer loopback socket ->
   kernel WireGuard listen port
5. `dual-nat` acceptance now passes end-to-end, including overlay ICMP after
   handshake

## Scope

This execution plan covers the near-term Phase 05 closure path:

1. replace one-shot STUN candidate collection with a long-lived punch runtime
2. add peer-observed source-candidate feedback
3. integrate the punch runtime into the existing hole-punch state machine
4. tighten acceptance and packet-level verification on `dual-nat`

It keeps Linux on the current kernel WireGuard ownership model and adds a
proxy layer to make same-port public send/receive explicit.

## Task List

### TASK-P05-01 Add peer-observed candidate feedback to `SignalService`

Commit title:

`phase05: add peer-observed candidate feedback to signal flow`

Goal:

1. allow a peer to report the real outer source `ip:port` it observed during a
   direct punch attempt
2. keep the protocol backward compatible

Files:

1. `proto/signal.proto`
2. `docs/api/service-contracts.md`
3. `docs/tasks/phase-05-nat-punching.md`
4. `docs/tasks/phase-05-dual-nat-root-cause.md`
5. generated Rust and Go proto outputs via `./scripts/gen-proto.sh`
6. `server/internal/signal/...` only if the generated API surface requires
   touch-ups beyond pure forwarding
7. `client/crates/agent-core/src/signal.rs`

Implementation notes:

1. prefer extending `PunchResult` with an optional observed candidate field over
   adding a brand-new RPC or stream message
2. keep `signald` as a pure forwarder; do not move path-selection state into
   the server
3. document that STUN-derived candidates are provisional and may be superseded
   by peer-observed candidates

Verification:

1. `./scripts/gen-proto.sh`
2. `cd server && go test ./...`
3. `cargo test --manifest-path client/Cargo.toml --workspace`

### TASK-P05-02 Introduce a long-lived client `PunchSocket` runtime

Commit title:

`phase05: add long-lived punch socket runtime`

Goal:

1. create one persistent UDP socket for punch-related discovery and probing
2. stop relying on short-lived STUN sockets for every candidate refresh

Files:

1. `client/crates/agent-core/src/punch_socket.rs`
2. `client/crates/agent-core/src/lib.rs`
3. `client/crates/agent-core/src/signal.rs`
4. `client/crates/agent-core/Cargo.toml`
5. optional new tests under `client/crates/agent-core/tests/...` if that layout
   is preferred over inline tests

Implementation notes:

1. the runtime should own one `tokio::net::UdpSocket` bound to the configured
   punch/listen port
2. it should expose:
   `query_stun`, `send_probe`, `recv_packet`, and runtime state inspection
3. this commit should only add the runtime scaffold and tests; it should not
   yet change the Phase 05 success criteria

Verification:

1. `cargo test --manifest-path client/Cargo.toml -p agent-core`
2. `cargo fmt --manifest-path client/Cargo.toml --all`

### TASK-P05-03 Switch public candidate collection to the long-lived `PunchSocket`

Commit title:

`phase05: source public candidates from punch socket`

Goal:

1. make the advertised public candidate come from the persistent punch runtime
2. remove the dependence on one-shot STUN sockets in `refresh_local_candidates`

Files:

1. `client/crates/stun/src/lib.rs`
2. `client/crates/agent-core/src/punch_socket.rs`
3. `client/crates/agent-core/src/signal.rs`
4. `client/crates/agent-core/src/lib.rs`
5. `client/crates/stun/Cargo.toml` only if test helpers or new features are
   needed

Implementation notes:

1. add a `stun::query_on_socket(...)` style API that reuses an existing UDP
   socket
2. keep the old STUN helpers temporarily for compatibility and tests
3. mark the resulting candidate with a distinct source such as
   `punch-socket-stun` instead of `stun`
4. do not remove cached-candidate fallback behavior yet

Verification:

1. `cargo test --manifest-path client/Cargo.toml -p stun`
2. `cargo test --manifest-path client/Cargo.toml -p agent-core`
3. `bash -n tests/nat-lab/run-phase05.sh`

### TASK-P05-04 Wire peer-observed source candidate feedback into client convergence

Commit title:

`phase05: promote peer-observed source candidate into convergence flow`

Goal:

1. let a client accept the real source candidate observed by its peer
2. re-announce the corrected candidate fast enough to matter during the punch
   window

Files:

1. `client/crates/agent-core/src/signal.rs`
2. `client/crates/holepunch/src/lib.rs`
3. `proto/signal.proto` only if a final field-shape adjustment is still needed
4. `docs/api/service-contracts.md` if semantic wording changes during
   implementation

Implementation notes:

1. define candidate replacement rules explicitly:
   public candidate from peer observation should supersede provisional STUN data
2. keep the logic in `agent-core`; `holepunch` should stay focused on candidate
   ranking and path-selection helpers
3. log the before/after endpoint so packet captures can be correlated with
   runtime events

Verification:

1. `cargo test --manifest-path client/Cargo.toml -p holepunch`
2. `cargo test --manifest-path client/Cargo.toml -p agent-core`
3. local log validation that observed candidates trigger re-announce behavior

### TASK-P05-05 Drive punch attempts through `PunchSocket` and tighten success rules

Commit title:

`phase05: send direct probes through punch socket and tighten convergence`

Goal:

1. have the punch runtime actively send direct probes during Phase 05 attempts
2. stop treating endpoint override alone as sufficient progress
3. require stronger evidence before declaring direct-path success

Files:

1. `client/crates/agent-core/src/punch_socket.rs`
2. `client/crates/agent-core/src/signal.rs`
3. `client/crates/holepunch/src/lib.rs`
4. `client/crates/netlink-linux/src/lib.rs` only if extra Linux-side endpoint
   inspection is still useful for telemetry
5. `docs/tasks/phase-05-dual-nat-root-cause.md`

Implementation notes:

1. `start_attempts()` should send probes through `PunchSocket`, not only update
   WireGuard endpoint overrides
2. direct success should require a stable corrected candidate and either
   handshake or other direct-path confirmation
3. the older runtime-UAPI endpoint observation path should become secondary
   telemetry, not the primary closure mechanism

Verification:

1. `cargo test --manifest-path client/Cargo.toml -p agent-core`
2. `cargo test --manifest-path client/Cargo.toml -p netlink-linux`
3. `MESHLINK_LAB_TOPOLOGY=dual-nat ./tests/nat-lab/run-phase05.sh`

### TASK-P05-06 Harden acceptance, lab checks, and packet-level regression rules

Commit title:

`phase05: harden dual-nat acceptance around corrected candidate convergence`

Goal:

1. make the acceptance path explicitly verify the corrected-candidate flow
2. prevent regressions where logs look healthy but WAN packets still show port
   drift

Files:

1. `tests/nat-lab/run-phase05.sh`
2. `tests/nat-lab/common.sh` only if helper functions are needed
3. `docs/tasks/phase-05-nat-punching.md`
4. `docs/tasks/phase-05-dual-nat-root-cause.md`
5. `docs/tasks/phase-05-explicit-punch-socket-execution.md`

Implementation notes:

1. strengthen log assertions so they cover observed-candidate correction rather
   than only generic success strings
2. where practical, collect `wg show` endpoint state and NAT-WAN packet traces
   into runtime artifacts
3. keep the scripted acceptance compact; packet capture can remain a targeted
   debug aid rather than a permanent heavy-weight step

Verification:

1. `bash -n tests/nat-lab/common.sh tests/nat-lab/run-phase05.sh`
2. `MESHLINK_LAB_TOPOLOGY=dual-nat ./tests/nat-lab/run-phase05.sh`
3. targeted NAT-WAN capture confirming that the final announced candidate port
   matches the observed direct-path source port in both directions

## Sequencing Rules

1. `TASK-P05-01` must land before any runtime code that depends on the new
   field semantics
2. `TASK-P05-02` and `TASK-P05-03` can be reviewed as the runtime foundation
   before the correction loop is introduced
3. `TASK-P05-04` is the first commit that should materially change Phase 05
   convergence behavior
4. `TASK-P05-05` is the commit expected to close the direct-path acceptance gap
5. `TASK-P05-06` should land only after the new behavior is stable enough to
   encode in scripted acceptance

## Out Of Scope

The following work should not be folded into this six-commit sequence:

1. replacing Linux kernel WireGuard ownership with a fully userspace-owned
   transport socket
2. large relay protocol redesign
3. Windows-specific path changes
4. admin UI or management-plane feature changes unrelated to candidate
   convergence
