# Phase 05 Dual-NAT Root Cause Analysis

## Goal

Document the concrete failure mode behind the current Phase 05 `dual-nat`
acceptance gap and turn the packet evidence into a repair-oriented problem
statement.

## Scope

This document covers the original failure of
`MESHLINK_LAB_TOPOLOGY=dual-nat ./tests/nat-lab/run-phase05.sh` on the Linux
VM lab and the final fix that closed it on 2026-04-20.

It does not redefine the Phase 05 feature goal. It explains why the current
implementation does not reliably converge to a direct path even though:

1. `signald` candidate exchange works
2. STUN returns public mappings
3. direct peer-to-peer UDP packets are visible on the wire

Current repo status on 2026-04-20:

1. protocol support for peer-observed candidates is now present
2. client-side `PunchSocket` runtime is now integrated with a Linux public UDP
   proxy that owns the public same-port socket
3. kernel WireGuard stays in place, but now listens on an internal loopback
   port behind that proxy
4. `dual-nat` acceptance now passes after the proxy mapping bug in
   `remote_direct_progress` was removed

## Resolution Summary

The original dual-NAT failure had two layers:

1. kernel WireGuard could not safely share the public listen port with a
   userspace same-port punch socket
2. after introducing the proxy-based solution, the final blocker was a
   convergence bug: `PunchResult.selected_candidate` was interpreted as the
   peer endpoint that the local node should dial, even though it actually
   describes the candidate that the peer selected for the local node

That second bug caused both proxies to rewrite their per-peer remote mapping to
the wrong endpoint immediately after the first successful handshake. The direct
handshake log appeared, but post-handshake encrypted packets were dropped with:

1. `dropping public udp packet with no matching peer remote_addr=...`
2. `wg show` showing TX bytes advancing while RX bytes stayed at `0 B`
3. overlay pings still failing despite `hole punch handshake observed`

The final fix was:

1. keep proxy remote-endpoint state anchored to the locally selected remote
   candidate or the locally observed remote candidate
2. stop replacing that state from the peer's echoed `selected_candidate`
3. continue using `observed_candidate` only as a correction channel for the
   local node's own public candidate

After that fix, `dual-nat` acceptance completed with:

1. `hole punch handshake observed` on both peers
2. no post-handshake proxy peer-matching drops
3. successful overlay ICMP between `100.64.0.1` and `100.64.0.2`

## Failure Summary

Observed acceptance failure:

1. `run-phase05.sh` times out waiting for
   `hole punch handshake observed`
2. both peers eventually keep working over relay instead of stabilizing on a
   direct NAT-WAN path
3. NAT router counters show only one or two direct packets hitting
   `51820/51821` during the whole punch window, which means the current
   kernel-WG path is not producing a sustained direct handshake exchange

The concrete defect is:

1. the candidate port announced through `SignalService` is not always the same
   as the real UDP source port used by the direct data path
2. the peer therefore learns or targets the wrong endpoint during punching
3. the direct path becomes unstable and Phase 05 falls back to relay

## Environment

Relevant addresses from the reproduced lab run:

1. management node: `192.168.123.201`
2. NAT A WAN: `192.168.123.211`
3. NAT B WAN: `192.168.123.221`
4. client A LAN: `10.10.1.10`
5. client B LAN: `10.10.2.10`

Expected public candidates for the test run:

1. client A candidate: `192.168.123.211:51820`
2. client B candidate: `192.168.123.221:51821`

## Capture Method

Packet captures were taken on the NAT WAN and LAN interfaces with:

```bash
udp and (port 51820 or port 51821 or port 3478 or port 3479)
```

Primary views used for the analysis:

1. `nat-a-wan`
2. `nat-b-wan`
3. `nat-b-lan`

Useful Wireshark display filters for the same evidence:

```text
udp.port == 3479
udp.port == 51820 || udp.port == 51821
ip.addr == 192.168.123.201 && (udp.port == 3478 || udp.port == 51820 || udp.port == 51821)
```

## Packet Evidence

### 1. STUN succeeds

Public mapping discovery is functioning. For example on NAT B WAN:

1. `2026-04-20 09:49:08.587278`
   `192.168.123.221:51821 -> 192.168.123.201:3479`
2. `2026-04-20 09:49:08.587451`
   `192.168.123.201:3479 -> 192.168.123.221:51821`

This rules out a simple "STUN is broken" explanation.

### 2. Direct peer-to-peer UDP is attempted

The peers do send packets directly across NAT WAN addresses.

On NAT B WAN:

1. `2026-04-20 09:49:11.290731`
   `192.168.123.221:51821 -> 192.168.123.211:51820`

On NAT A WAN:

1. `2026-04-20 09:49:11.284776`
   `192.168.123.221:51821 -> 192.168.123.211:51820`

This rules out a simple "clients never attempt direct punching" explanation.

### 3. Reverse direct packets arrive from the wrong source port

The decisive evidence is that the returning direct packet from client A does
not preserve the announced source port `51820`.

On NAT B WAN:

1. announced target was `192.168.123.211:51820`
2. actual return packet at `2026-04-20 09:49:11.291368` was
   `192.168.123.211:23334 -> 192.168.123.221:51821`
3. another direct packet at `2026-04-20 09:49:16.793960` was again
   `192.168.123.211:23334 -> 192.168.123.221:51821`
4. later the source port drifted again at `2026-04-20 09:50:21.288432` to
   `192.168.123.211:29976 -> 192.168.123.221:51821`

The same mismatch is visible from NAT A WAN:

1. `2026-04-20 09:49:11.285285`
   `192.168.123.211:23334 -> 192.168.123.221:51821`
2. `2026-04-20 09:50:21.282358`
   `192.168.123.211:29976 -> 192.168.123.221:51821`

This is the concrete break in the Phase 05 closure path.

### 4. Relay remains healthy

Once direct convergence fails, both peers continue over relay. Example relay
traffic:

1. `192.168.123.211:51820 <-> 192.168.123.201:54445`
2. `192.168.123.221:51821 <-> 192.168.123.201:54445`

This explains why connectivity survives while direct acceptance still fails.

### 5. ICMP unreachable on stale relay ports is secondary noise

Packets such as:

1. `192.168.123.211:51820 -> 192.168.123.201:43793`
2. `192.168.123.221:51821 -> 192.168.123.201:43793`

received ICMP port-unreachable replies from the management node. This indicates
old relay or reservation ports being retried after the server side closed them.
That behavior should be cleaned up, but it is not the primary cause of the
direct-path failure.

## Original Root Cause

The current Phase 05 design assumes that:

1. the public candidate discovered through STUN
2. the port announced through `SignalService`
3. the real outer UDP source port used by the direct path

are the same port.

The packet captures show that this assumption is false in the current Linux
WireGuard path under the reproduced dual-NAT environment.

More precisely:

1. the system announces `192.168.123.211:51820` as client A's public candidate
2. client B therefore sends direct traffic to `192.168.123.211:51820`
3. client A's return traffic reaches client B from a different port such as
   `23334` or `29976`
4. the peer now observes a source endpoint that disagrees with the signaled
   candidate
5. endpoint convergence and handshake establishment do not stabilize

That was the original failure mode for the explicit punch-socket branch.

After switching to the current kernel-WG-only observed-source path, the
reproduced dual-NAT runs now fail slightly earlier:

1. bootstrap STUN still advertises the expected WAN `ip:port`
2. both peers still select the NAT WAN candidate and start a punch attempt
3. NAT routers see at most one inbound direct WireGuard initiation packet and
   a handful of outbound packets during the whole attempt window
4. no peer reaches `hole punch handshake observed`
5. both peers time out and fall back to relay

That was the original breakage before the proxy-based same-port data plane was
introduced.

## Why Earlier Mitigations Did Not Close The Gap

Several earlier mitigations improved diagnostics but did not remove the root
cause:

1. relaxing punch timeout only gives the wrong endpoint more time
2. accepting weaker success log markers does not change the actual packet
   source port seen by the peer
3. delaying signal startup can reduce races, but it cannot fix a mismatched
   transport port
4. trying to derive candidates from WireGuard runtime state did not trigger in
   the reproduced run because the mismatched direct-path source port was not
   surfaced to that code path before timeout
5. moving observed-source detection to WireGuard runtime telemetry removed the
   broken userspace same-port receive dependency, but it also exposed that the
   kernel-WG-only trigger path currently produces too little direct traffic to
   reach handshake success reliably

## Repair Direction

There are two viable repair classes.

### Option A: explicit punch socket

Create and control a dedicated UDP punch socket whose bound local port is the
same port that gets advertised as the public candidate.

Implications:

1. the implementation owns the port that STUN observes
2. the same socket can send the punch packets
3. the signaled candidate and the real direct-path source port stay aligned

This is the most direct way to make the advertised candidate truthful.

### Option B: advertise only the real observed source port

Continue using the current data path, but promote candidate publication only
after observing the real outer source port from the transport actually used for
direct traffic.

### Option C: add a kernel-compatible direct trigger path

Keep kernel WireGuard as the port owner, but add a trigger path that can make
the kernel emit a real sequence of direct handshake attempts instead of a
single initiation packet.

Current evidence says that simple endpoint reapply, endpoint clear/set, and
overlay ping bursts are still insufficient in the reproduced lab.

Implications:

1. STUN alone is no longer authoritative
2. the client needs a reliable lower-level observation path for the real source
   port
3. candidate updates must be propagated quickly enough to matter during the
   punch window

This is harder operationally because it depends on reliable runtime observation
of a port that the current higher-level code did not see during the reproduced
failure.

## Recommended Fix

For Phase 05 closure, prefer Option A.

Reason:

1. it removes the ambiguity between discovery and transport
2. it avoids depending on kernel or WireGuard runtime behavior that is not
   currently exposed in time to repair the candidate
3. it makes acceptance criteria testable with a simple packet invariant:
   "the announced candidate port equals the observed direct-path source port"

## Acceptance Criteria For The Fix

Phase 05 should not be considered closed again until all of the following are
true on `dual-nat`:

1. the port observed by STUN matches the port used by the direct punch traffic
2. WAN captures show both directions using the announced candidate ports
3. `run-phase05.sh` observes direct-path success without falling back to relay
4. post-convergence `wg` peer endpoint state matches the direct NAT-WAN path

## Follow-Up Validation

After the repair lands, re-run:

1. `MESHLINK_LAB_TOPOLOGY=dual-nat ./tests/nat-lab/run-phase05.sh`
2. packet captures on both NAT WAN interfaces
3. `wg` peer endpoint inspection on both clients after direct convergence

The expected invariant in packet capture should look like:

1. `192.168.123.221:51821 -> 192.168.123.211:51820`
2. `192.168.123.211:51820 -> 192.168.123.221:51821`

without source-port drift to random high ports.

The concrete implementation sequence for that repair is tracked in
`docs/tasks/phase-05-explicit-punch-socket-execution.md`.
