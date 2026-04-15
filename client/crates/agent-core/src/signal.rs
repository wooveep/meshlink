use std::{
    collections::{BTreeMap, BTreeSet},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use api_client::proto::{
    signal_envelope::Body, signal_service_client::SignalServiceClient, Candidate,
    CandidateAnnouncement, CandidateType, Heartbeat, PunchRequest, PunchResult, SignalEnvelope,
    SignalHello, SignalKind,
};
use holepunch::{
    candidate_path_type, select_remote_candidate_for_local, should_initiate, sort_candidates,
};
use netlink_linux::latest_handshake_timestamp;
use relay_client::{release_peer_relay, reserve_peer_relay, RelayReservation};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
    time::{interval, sleep},
};
use tokio_stream::{wrappers::ReceiverStream, StreamExt};
use tracing::{info, warn};
use wg_manager::Endpoint;

use crate::{api_client_endpoint, AgentConfig, CachedPeer, PeerSnapshot};

#[derive(Debug, Clone)]
pub struct SignalUpdate {
    pub peer_id: String,
    pub endpoint_override: Option<Endpoint>,
    pub probe_overlay_ipv4: Option<String>,
    pub reason: String,
}

pub struct SignalRuntime {
    snapshot_tx: watch::Sender<PeerSnapshot>,
    update_rx: mpsc::Receiver<SignalUpdate>,
    task: JoinHandle<()>,
}

impl SignalRuntime {
    pub fn spawn(config: AgentConfig, device_id: String) -> Option<Self> {
        let signal_addr = config.signal_addr.clone()?;
        let (snapshot_tx, snapshot_rx) = watch::channel(PeerSnapshot::default());
        let (update_tx, update_rx) = mpsc::channel(64);

        let task = tokio::spawn(async move {
            run_signal_loop(config, device_id, signal_addr, snapshot_rx, update_tx).await;
        });

        Some(Self {
            snapshot_tx,
            update_rx,
            task,
        })
    }

    pub fn publish_snapshot(&self, snapshot: PeerSnapshot) {
        let _ = self.snapshot_tx.send(snapshot);
    }

    pub async fn recv_update(&mut self) -> Option<SignalUpdate> {
        self.update_rx.recv().await
    }
}

impl Drop for SignalRuntime {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Debug, Default)]
struct PeerSignalState {
    remote_candidates: Vec<Candidate>,
    request_received: bool,
    attempt: Option<PunchAttempt>,
    relay: Option<ActiveRelay>,
}

#[derive(Debug, Clone)]
struct PunchAttempt {
    selected_candidate: Candidate,
    started_at: Instant,
    baseline_handshake: u64,
}

#[derive(Debug, Clone)]
struct ActiveRelay {
    reservation: RelayReservation,
    next_refresh_at: Instant,
}

impl ActiveRelay {
    fn new(reservation: RelayReservation) -> Self {
        Self {
            next_refresh_at: Instant::now() + relay_refresh_delay(reservation.ttl_seconds),
            reservation,
        }
    }

    fn endpoint(&self, relay_addr: &str) -> Endpoint {
        Endpoint {
            host: relay_host(&self.reservation, relay_addr),
            port: self.reservation.udp_port,
        }
    }

    fn refresh_due(&self) -> bool {
        Instant::now() >= self.next_refresh_at
    }

    fn refresh_from_reservation(&mut self, reservation: RelayReservation) {
        self.next_refresh_at = Instant::now() + relay_refresh_delay(reservation.ttl_seconds);
        self.reservation = reservation;
    }
}

async fn run_signal_loop(
    config: AgentConfig,
    device_id: String,
    signal_addr: String,
    snapshot_rx: watch::Receiver<PeerSnapshot>,
    update_tx: mpsc::Sender<SignalUpdate>,
) {
    loop {
        match connect_and_run(
            &config,
            &device_id,
            &signal_addr,
            snapshot_rx.clone(),
            update_tx.clone(),
        )
        .await
        {
            Ok(()) => return,
            Err(err) => {
                warn!("signal loop disconnected: {err:#}");
                sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

async fn connect_and_run(
    config: &AgentConfig,
    device_id: &str,
    signal_addr: &str,
    mut snapshot_rx: watch::Receiver<PeerSnapshot>,
    update_tx: mpsc::Sender<SignalUpdate>,
) -> Result<()> {
    let endpoint = api_client_endpoint(signal_addr);
    let mut client = SignalServiceClient::connect(endpoint)
        .await
        .context("connect signal service")?;

    let (outbound_tx, outbound_rx) = mpsc::channel(64);
    let hello = SignalEnvelope {
        kind: SignalKind::Hello as i32,
        source_device_id: device_id.to_string(),
        body: Some(Body::Hello(SignalHello {
            device_id: device_id.to_string(),
            public_key: config.public_key.clone(),
            bootstrap_token: config.bootstrap_token.clone(),
        })),
        ..Default::default()
    };

    let response = client
        .open_signal(tokio_stream::once(hello).chain(ReceiverStream::new(outbound_rx)))
        .await
        .context("open signal stream")?;
    let mut inbound = response.into_inner();
    info!("signal stream connected");

    let local_candidates = collect_local_candidates(config).await;
    info!(
        candidates = local_candidates.len(),
        "local punch candidates collected"
    );
    let mut announce_tick = interval(Duration::from_secs(5));
    let mut heartbeat_tick = interval(Duration::from_secs(5));
    let mut punch_tick = interval(Duration::from_millis(500));
    let mut relay_tick = interval(Duration::from_secs(1));
    let mut peer_states = BTreeMap::<String, PeerSignalState>::new();

    let initial_snapshot = snapshot_rx.borrow().clone();
    announce_candidates(
        &outbound_tx,
        device_id,
        &initial_snapshot,
        &local_candidates,
    )
    .await?;

    let result: Result<()> = async {
        loop {
            tokio::select! {
                changed = snapshot_rx.changed() => {
                    changed.context("receive peer snapshot")?;
                    let snapshot = snapshot_rx.borrow().clone();
                    prune_removed_peers(config, device_id, &snapshot, &mut peer_states, &update_tx).await?;
                    announce_candidates(&outbound_tx, device_id, &snapshot, &local_candidates).await?;
                    maybe_send_punch_requests(&outbound_tx, device_id, &snapshot, &local_candidates, &peer_states).await?;
                    start_attempts(
                        config,
                        device_id,
                        &snapshot,
                        &mut peer_states,
                        &update_tx,
                        &local_candidates,
                    ).await?;
                }
                _ = heartbeat_tick.tick() => {
                    outbound_tx.send(SignalEnvelope {
                        kind: SignalKind::Heartbeat as i32,
                        source_device_id: device_id.to_string(),
                        body: Some(Body::Heartbeat(Heartbeat {})),
                        ..Default::default()
                    }).await.context("send signal heartbeat")?;
                }
                _ = announce_tick.tick() => {
                    let snapshot = snapshot_rx.borrow().clone();
                    announce_candidates(&outbound_tx, device_id, &snapshot, &local_candidates).await?;
                    maybe_send_punch_requests(&outbound_tx, device_id, &snapshot, &local_candidates, &peer_states).await?;
                    start_attempts(
                        config,
                        device_id,
                        &snapshot,
                        &mut peer_states,
                        &update_tx,
                        &local_candidates,
                    ).await?;
                }
                _ = punch_tick.tick() => {
                    let snapshot = snapshot_rx.borrow().clone();
                    check_attempts(
                        config,
                        device_id,
                        &snapshot,
                        &outbound_tx,
                        &mut peer_states,
                        &update_tx,
                    ).await?;
                }
                _ = relay_tick.tick() => {
                    let snapshot = snapshot_rx.borrow().clone();
                    refresh_relays(config, device_id, &snapshot, &mut peer_states, &update_tx).await?;
                }
                inbound_message = inbound.message() => {
                    let Some(envelope) = inbound_message.context("receive signal message")? else {
                        break Ok(());
                    };
                    let snapshot = snapshot_rx.borrow().clone();
                    handle_incoming_envelope(
                        config,
                        device_id,
                        &snapshot,
                        &local_candidates,
                        envelope,
                        &outbound_tx,
                        &mut peer_states,
                        &update_tx,
                    ).await?;
                }
            }
        }
    }
    .await;

    cleanup_relay_sessions(config, device_id, &mut peer_states).await;
    result
}

async fn handle_incoming_envelope(
    config: &AgentConfig,
    device_id: &str,
    snapshot: &PeerSnapshot,
    local_candidates: &[Candidate],
    envelope: SignalEnvelope,
    outbound_tx: &mpsc::Sender<SignalEnvelope>,
    peer_states: &mut BTreeMap<String, PeerSignalState>,
    update_tx: &mpsc::Sender<SignalUpdate>,
) -> Result<()> {
    let Some(peer) = snapshot
        .peers
        .iter()
        .find(|peer| peer.peer_id == envelope.source_device_id)
    else {
        return Ok(());
    };

    let state = peer_states.entry(peer.peer_id.clone()).or_default();
    match envelope.body {
        Some(Body::CandidateAnnouncement(body)) => {
            info!(
                peer_id = %peer.peer_id,
                candidates = body.candidates.len(),
                "received candidate announcement"
            );
            state.remote_candidates = sort_candidates(body.candidates);
            if should_initiate(device_id, &peer.peer_id) {
                send_punch_request(
                    outbound_tx,
                    device_id,
                    &peer.peer_id,
                    local_candidates.to_vec(),
                )
                .await?;
            }
            start_attempt_for_peer(config, device_id, peer, state, update_tx, local_candidates)
                .await?;
        }
        Some(Body::PunchRequest(body)) => {
            info!(
                peer_id = %peer.peer_id,
                candidates = body.candidates.len(),
                "received punch request"
            );
            state.request_received = true;
            if !body.candidates.is_empty() {
                state.remote_candidates = sort_candidates(body.candidates);
            }
            start_attempt_for_peer(config, device_id, peer, state, update_tx, local_candidates)
                .await?;
        }
        Some(Body::PunchResult(body)) => {
            info!(peer_id = %peer.peer_id, success = body.success, "received punch result");
            if !body.success {
                state.attempt = None;
                fallback_to_relay(
                    config,
                    device_id,
                    peer,
                    state,
                    update_tx,
                    "remote_punch_failed",
                )
                .await?;
            }
        }
        Some(Body::Heartbeat(_)) | Some(Body::Hello(_)) | None => {}
    }

    Ok(())
}

async fn prune_removed_peers(
    config: &AgentConfig,
    device_id: &str,
    snapshot: &PeerSnapshot,
    peer_states: &mut BTreeMap<String, PeerSignalState>,
    update_tx: &mpsc::Sender<SignalUpdate>,
) -> Result<()> {
    let visible = snapshot
        .peers
        .iter()
        .map(|peer| peer.peer_id.clone())
        .collect::<BTreeSet<_>>();
    let removed = peer_states
        .keys()
        .filter(|peer_id| !visible.contains(*peer_id))
        .cloned()
        .collect::<Vec<_>>();
    for peer_id in removed {
        if let Some(mut state) = peer_states.remove(&peer_id) {
            if let Some(active_relay) = state.relay.take() {
                release_active_relay(config, device_id, &peer_id, active_relay, "peer_removed")
                    .await;
            }
        }
        update_tx
            .send(SignalUpdate {
                peer_id,
                endpoint_override: None,
                probe_overlay_ipv4: None,
                reason: "peer_removed".to_string(),
            })
            .await
            .context("clear endpoint override for removed peer")?;
    }
    Ok(())
}

async fn announce_candidates(
    outbound_tx: &mpsc::Sender<SignalEnvelope>,
    device_id: &str,
    snapshot: &PeerSnapshot,
    local_candidates: &[Candidate],
) -> Result<()> {
    if local_candidates.is_empty() {
        return Ok(());
    }

    for peer in &snapshot.peers {
        outbound_tx
            .send(SignalEnvelope {
                kind: SignalKind::Candidates as i32,
                source_device_id: device_id.to_string(),
                target_device_id: peer.peer_id.clone(),
                session_id: session_id(device_id, &peer.peer_id),
                body: Some(Body::CandidateAnnouncement(CandidateAnnouncement {
                    candidates: local_candidates.to_vec(),
                })),
            })
            .await
            .context("send candidate announcement")?;
        info!(
            peer_id = %peer.peer_id,
            candidates = local_candidates.len(),
            "sent candidate announcement"
        );
    }
    Ok(())
}

async fn maybe_send_punch_requests(
    outbound_tx: &mpsc::Sender<SignalEnvelope>,
    device_id: &str,
    snapshot: &PeerSnapshot,
    local_candidates: &[Candidate],
    peer_states: &BTreeMap<String, PeerSignalState>,
) -> Result<()> {
    for peer in &snapshot.peers {
        let Some(state) = peer_states.get(&peer.peer_id) else {
            continue;
        };
        if !should_initiate(device_id, &peer.peer_id) || state.remote_candidates.is_empty() {
            continue;
        }
        send_punch_request(
            outbound_tx,
            device_id,
            &peer.peer_id,
            local_candidates.to_vec(),
        )
        .await?;
    }
    Ok(())
}

async fn send_punch_request(
    outbound_tx: &mpsc::Sender<SignalEnvelope>,
    device_id: &str,
    peer_id: &str,
    local_candidates: Vec<Candidate>,
) -> Result<()> {
    let candidate_count = local_candidates.len();
    outbound_tx
        .send(SignalEnvelope {
            kind: SignalKind::PunchRequest as i32,
            source_device_id: device_id.to_string(),
            target_device_id: peer_id.to_string(),
            session_id: session_id(device_id, peer_id),
            body: Some(Body::PunchRequest(PunchRequest {
                candidates: local_candidates,
            })),
        })
        .await
        .context("send punch request")?;
    info!(peer_id = %peer_id, candidates = candidate_count, "sent punch request");
    Ok(())
}

async fn start_attempts(
    config: &AgentConfig,
    device_id: &str,
    snapshot: &PeerSnapshot,
    peer_states: &mut BTreeMap<String, PeerSignalState>,
    update_tx: &mpsc::Sender<SignalUpdate>,
    local_candidates: &[Candidate],
) -> Result<()> {
    for peer in &snapshot.peers {
        if let Some(state) = peer_states.get_mut(&peer.peer_id) {
            start_attempt_for_peer(config, device_id, peer, state, update_tx, local_candidates)
                .await?;
        }
    }
    Ok(())
}

async fn start_attempt_for_peer(
    config: &AgentConfig,
    device_id: &str,
    peer: &CachedPeer,
    state: &mut PeerSignalState,
    update_tx: &mpsc::Sender<SignalUpdate>,
    local_candidates: &[Candidate],
) -> Result<()> {
    if state.attempt.is_some() {
        return Ok(());
    }
    if state.remote_candidates.is_empty() {
        return Ok(());
    }
    if !state.request_received && !should_initiate(device_id, &peer.peer_id) {
        return Ok(());
    }

    let Some(candidate) =
        select_remote_candidate_for_local(&state.remote_candidates, local_candidates)
    else {
        return Ok(());
    };
    let Ok(port) = u16::try_from(candidate.port) else {
        return Ok(());
    };

    let endpoint = Endpoint {
        host: candidate.address.clone(),
        port,
    };
    let baseline = latest_handshake(&config.interface_name, &peer.public_key).unwrap_or_default();

    state.attempt = Some(PunchAttempt {
        selected_candidate: candidate,
        started_at: Instant::now(),
        baseline_handshake: baseline,
    });
    info!(
        peer_id = %peer.peer_id,
        endpoint = %endpoint.render(),
        "starting punch attempt"
    );

    update_tx
        .send(SignalUpdate {
            peer_id: peer.peer_id.clone(),
            endpoint_override: Some(endpoint),
            probe_overlay_ipv4: Some(peer.overlay_ipv4.clone()),
            reason: "punch_started".to_string(),
        })
        .await
        .context("send endpoint override update")
}

async fn check_attempts(
    config: &AgentConfig,
    device_id: &str,
    snapshot: &PeerSnapshot,
    outbound_tx: &mpsc::Sender<SignalEnvelope>,
    peer_states: &mut BTreeMap<String, PeerSignalState>,
    update_tx: &mpsc::Sender<SignalUpdate>,
) -> Result<()> {
    for peer in &snapshot.peers {
        let Some(state) = peer_states.get_mut(&peer.peer_id) else {
            continue;
        };
        let Some(attempt) = state.attempt.clone() else {
            continue;
        };

        let latest = latest_handshake(&config.interface_name, &peer.public_key).unwrap_or_default();
        if latest > attempt.baseline_handshake && latest > 0 {
            info!(peer_id = %peer.peer_id, "hole punch handshake observed");
            outbound_tx
                .send(SignalEnvelope {
                    kind: SignalKind::PunchResult as i32,
                    source_device_id: device_id.to_string(),
                    target_device_id: peer.peer_id.clone(),
                    session_id: session_id(device_id, &peer.peer_id),
                    body: Some(Body::PunchResult(PunchResult {
                        success: true,
                        selected_candidate: Some(attempt.selected_candidate.clone()),
                        path_type: candidate_path_type(&attempt.selected_candidate) as i32,
                        reason: "handshake_observed".to_string(),
                    })),
                })
                .await
                .context("send punch success")?;
            state.attempt = None;
            if let Some(active_relay) = state.relay.take() {
                info!(peer_id = %peer.peer_id, "direct path recovered; releasing relay");
                release_active_relay(
                    config,
                    device_id,
                    &peer.peer_id,
                    active_relay,
                    "direct_recovered",
                )
                .await;
            }
            continue;
        }

        probe_overlay_peer(&peer.overlay_ipv4);

        if attempt.started_at.elapsed() >= config.punch_timeout {
            warn!(peer_id = %peer.peer_id, "hole punch attempt timed out");
            outbound_tx
                .send(SignalEnvelope {
                    kind: SignalKind::PunchResult as i32,
                    source_device_id: device_id.to_string(),
                    target_device_id: peer.peer_id.clone(),
                    session_id: session_id(device_id, &peer.peer_id),
                    body: Some(Body::PunchResult(PunchResult {
                        success: false,
                        selected_candidate: Some(attempt.selected_candidate.clone()),
                        path_type: candidate_path_type(&attempt.selected_candidate) as i32,
                        reason: "timeout".to_string(),
                    })),
                })
                .await
                .context("send punch failure")?;
            state.attempt = None;
            fallback_to_relay(config, device_id, peer, state, update_tx, "punch_timeout").await?;
        }
    }
    Ok(())
}

async fn refresh_relays(
    config: &AgentConfig,
    device_id: &str,
    snapshot: &PeerSnapshot,
    peer_states: &mut BTreeMap<String, PeerSignalState>,
    update_tx: &mpsc::Sender<SignalUpdate>,
) -> Result<()> {
    let Some(relay_addr) = config.relay_addr.as_deref() else {
        return Ok(());
    };

    for peer in &snapshot.peers {
        let Some(state) = peer_states.get_mut(&peer.peer_id) else {
            continue;
        };
        let Some(active_relay) = state.relay.as_mut() else {
            continue;
        };
        if !active_relay.refresh_due() {
            continue;
        }

        match reserve_peer_relay(
            relay_addr,
            device_id,
            &config.public_key,
            &config.bootstrap_token,
            &peer.peer_id,
        )
        .await
        {
            Ok(reservation) => {
                let previous_endpoint = active_relay.endpoint(relay_addr);
                active_relay.refresh_from_reservation(reservation);
                let refreshed_endpoint = active_relay.endpoint(relay_addr);

                if state.attempt.is_none() && refreshed_endpoint != previous_endpoint {
                    update_tx
                        .send(SignalUpdate {
                            peer_id: peer.peer_id.clone(),
                            endpoint_override: Some(refreshed_endpoint),
                            probe_overlay_ipv4: None,
                            reason: "relay_refreshed".to_string(),
                        })
                        .await
                        .context("apply refreshed relay endpoint")?;
                }
            }
            Err(err) => warn!(peer_id = %peer.peer_id, "refresh relay reservation failed: {err:#}"),
        }
    }

    Ok(())
}

async fn fallback_to_relay(
    config: &AgentConfig,
    device_id: &str,
    peer: &CachedPeer,
    state: &mut PeerSignalState,
    update_tx: &mpsc::Sender<SignalUpdate>,
    reason: &str,
) -> Result<()> {
    let endpoint_override = if let Some(relay_addr) = config.relay_addr.as_deref() {
        if state.relay.is_none() {
            match reserve_peer_relay(
                relay_addr,
                device_id,
                &config.public_key,
                &config.bootstrap_token,
                &peer.peer_id,
            )
            .await
            {
                Ok(reservation) => {
                    state.relay = Some(ActiveRelay::new(reservation));
                    info!(peer_id = %peer.peer_id, "relay fallback activated");
                }
                Err(err) => {
                    warn!(peer_id = %peer.peer_id, "reserve relay failed: {err:#}");
                }
            }
        }

        state
            .relay
            .as_ref()
            .map(|active_relay| active_relay.endpoint(relay_addr))
    } else {
        None
    };

    update_tx
        .send(SignalUpdate {
            peer_id: peer.peer_id.clone(),
            endpoint_override,
            probe_overlay_ipv4: None,
            reason: reason.to_string(),
        })
        .await
        .context("apply fallback endpoint override")
}

async fn cleanup_relay_sessions(
    config: &AgentConfig,
    device_id: &str,
    peer_states: &mut BTreeMap<String, PeerSignalState>,
) {
    for (peer_id, state) in peer_states.iter_mut() {
        if let Some(active_relay) = state.relay.take() {
            release_active_relay(
                config,
                device_id,
                peer_id,
                active_relay,
                "signal_loop_closed",
            )
            .await;
        }
    }
}

async fn release_active_relay(
    config: &AgentConfig,
    device_id: &str,
    peer_id: &str,
    active_relay: ActiveRelay,
    reason: &str,
) {
    let Some(relay_addr) = config.relay_addr.as_deref() else {
        return;
    };

    if let Err(err) = release_peer_relay(
        relay_addr,
        device_id,
        &config.public_key,
        &config.bootstrap_token,
        peer_id,
        &active_relay.reservation.session_id,
        reason,
    )
    .await
    {
        warn!(peer_id = %peer_id, "release relay failed: {err:#}");
    } else {
        info!(peer_id = %peer_id, reason, "relay reservation released");
    }
}

async fn collect_local_candidates(config: &AgentConfig) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    let listen_port = u32::from(config.listen_port.unwrap_or_default());
    if listen_port == 0 {
        return candidates;
    }

    for (address, interface_name) in collect_lan_ipv4s().unwrap_or_default() {
        candidates.push(Candidate {
            r#type: CandidateType::Lan as i32,
            address,
            port: listen_port,
            network_interface: interface_name,
            priority: 300,
        });
    }

    if let Some(host) = config.advertise_host.as_ref() {
        candidates.push(Candidate {
            r#type: CandidateType::PublicIpv4 as i32,
            address: host.clone(),
            port: listen_port,
            network_interface: "static".to_string(),
            priority: 200,
        });
    }

    if let Some(stun_addr) = config.resolved_stun_addr() {
        let mapping = match config.listen_port {
            Some(listen_port) => {
                stun::query_with_local_port(&stun_addr, Duration::from_secs(2), listen_port).await
            }
            None => stun::query(&stun_addr, Duration::from_secs(2)).await,
        };
        match mapping {
            Ok(mapping) => candidates.push(Candidate {
                r#type: CandidateType::PublicIpv4 as i32,
                address: mapping.address,
                port: u32::from(mapping.port),
                network_interface: "stun".to_string(),
                priority: 100,
            }),
            Err(err) => warn!("stun query failed: {err:#}"),
        }
    }

    dedupe_candidates(sort_candidates(candidates))
}

fn dedupe_candidates(candidates: Vec<Candidate>) -> Vec<Candidate> {
    let mut seen = BTreeSet::new();
    let mut deduped = Vec::new();
    for candidate in candidates {
        let key = (candidate.r#type, candidate.address.clone(), candidate.port);
        if seen.insert(key) {
            deduped.push(candidate);
        }
    }
    deduped
}

fn collect_lan_ipv4s() -> Result<Vec<(String, String)>> {
    let output = Command::new(resolve_ip_bin())
        .args(["-o", "-4", "addr", "show", "scope", "global"])
        .output()
        .context("run ip addr for candidate collection")?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    Ok(parse_ip_addr_output(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

fn parse_ip_addr_output(output: &str) -> Vec<(String, String)> {
    output
        .lines()
        .filter_map(|line| {
            let parts = line.split_whitespace().collect::<Vec<_>>();
            if parts.len() < 4 {
                return None;
            }
            let interface_name = parts[1].trim_end_matches(':').to_string();
            let cidr = parts[3];
            let address = cidr.split('/').next()?.to_string();
            if address.starts_with("127.") {
                return None;
            }
            Some((address, interface_name))
        })
        .collect()
}

fn resolve_ip_bin() -> &'static str {
    if std::path::Path::new("/usr/sbin/ip").exists() {
        "/usr/sbin/ip"
    } else {
        "ip"
    }
}

fn latest_handshake(interface_name: &str, peer_public_key: &str) -> Result<u64> {
    latest_handshake_timestamp(interface_name, peer_public_key)
}

fn session_id(device_id: &str, peer_id: &str) -> String {
    format!("{device_id}:{peer_id}")
}

fn relay_refresh_delay(ttl_seconds: u32) -> Duration {
    let ttl_seconds = ttl_seconds.max(2);
    Duration::from_secs(u64::from(ttl_seconds / 2))
}

fn relay_host(reservation: &RelayReservation, relay_addr: &str) -> String {
    if !reservation.relay_host.trim().is_empty() {
        return reservation.relay_host.clone();
    }

    let trimmed = relay_addr
        .trim_start_matches("http://")
        .trim_start_matches("https://");

    trimmed
        .rsplit_once(':')
        .map(|(host, _)| host.trim_matches(['[', ']']))
        .unwrap_or(trimmed)
        .to_string()
}

fn probe_overlay_peer(overlay_ipv4: &str) {
    if overlay_ipv4.trim().is_empty() {
        return;
    }

    let ping_bin = if std::path::Path::new("/usr/bin/ping").exists() {
        "/usr/bin/ping"
    } else {
        "ping"
    };

    let _ = Command::new(ping_bin)
        .args(["-c", "1", "-W", "1", overlay_ipv4])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use super::{parse_ip_addr_output, relay_host, relay_refresh_delay};
    use relay_client::RelayReservation;
    use std::time::Duration;

    #[test]
    fn parse_ip_addr_output_extracts_global_ipv4_addresses() {
        let parsed = parse_ip_addr_output(
            "2: eth0    inet 192.0.2.10/24 brd 192.0.2.255 scope global eth0\n3: lo    inet 127.0.0.1/8 scope host lo\n",
        );

        assert_eq!(parsed, vec![("192.0.2.10".to_string(), "eth0".to_string())]);
    }

    #[test]
    fn relay_refresh_delay_uses_half_ttl() {
        assert_eq!(relay_refresh_delay(30), Duration::from_secs(15));
        assert_eq!(relay_refresh_delay(1), Duration::from_secs(1));
    }

    #[test]
    fn relay_host_prefers_reservation_host_then_control_endpoint() {
        let explicit = RelayReservation {
            relay_host: "198.51.100.10".to_string(),
            udp_port: 45000,
            ttl_seconds: 30,
            session_id: "session".to_string(),
        };
        assert_eq!(relay_host(&explicit, "127.0.0.1:3478"), "198.51.100.10");

        let fallback = RelayReservation {
            relay_host: String::new(),
            udp_port: 45000,
            ttl_seconds: 30,
            session_id: "session".to_string(),
        };
        assert_eq!(
            relay_host(&fallback, "http://203.0.113.20:3478"),
            "203.0.113.20"
        );
    }
}
