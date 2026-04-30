use std::{
    collections::{BTreeMap, BTreeSet},
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use api_client::proto::{
    signal_envelope::Body, signal_service_client::SignalServiceClient, Candidate,
    CandidateAnnouncement, CandidateType, Heartbeat, PathType, PunchRequest, PunchResult,
    SignalEnvelope, SignalHello, SignalKind,
};
use holepunch::{
    candidate_path_type, select_remote_candidate_for_local, should_initiate, sort_candidates,
};
#[cfg(windows)]
use netlink_linux::ObservedRemoteEndpoint;
#[cfg(not(windows))]
use netlink_linux::{
    latest_handshake_timestamp, peer_endpoint as current_peer_endpoint, ObservedRemoteEndpoint,
};
use relay_client::{release_peer_relay, reserve_peer_relay, RelayReservation};
use serde::Deserialize;
use tokio::{
    sync::{mpsc, watch, Mutex},
    task::JoinHandle,
    time::{interval, sleep},
};
use tokio_stream::{wrappers::ReceiverStream, StreamExt};
use tracing::{info, warn};
use wg_manager::Endpoint;
#[cfg(windows)]
use wintun_windows::{latest_handshake_timestamp, peer_endpoint as current_peer_endpoint};

use crate::punch_socket::PunchSocket;
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
    pub fn spawn(
        config: AgentConfig,
        device_id: String,
        initial_candidates: Vec<Candidate>,
        initial_punch_socket: Option<PunchSocket>,
        observed_rx: Option<mpsc::Receiver<ObservedRemoteEndpoint>>,
    ) -> Option<Self> {
        let signal_addr = config.signal_addr.clone()?;
        let (snapshot_tx, snapshot_rx) = watch::channel(PeerSnapshot::default());
        let (update_tx, update_rx) = mpsc::channel(64);

        let task = tokio::spawn(async move {
            run_signal_loop(
                config,
                device_id,
                signal_addr,
                snapshot_rx,
                update_tx,
                initial_candidates,
                initial_punch_socket,
                observed_rx,
            )
            .await;
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
    path_mode: PathMode,
    attempt_generation: u64,
    last_selected_candidate: Option<Candidate>,
    last_applied_endpoint: Option<Endpoint>,
    last_observed_remote_candidate: Option<Candidate>,
    last_reported_observed_remote_candidate: Option<Candidate>,
    relay: Option<ActiveRelay>,
    next_direct_retry_at: Option<Instant>,
    last_direct_confirmed_at: Option<Instant>,
    last_transition_reason: Option<String>,
    last_transition_at: Option<Instant>,
}

#[derive(Debug, Clone)]
struct PunchAttempt {
    generation: u64,
    selected_candidate: Candidate,
    started_at: Instant,
    baseline_handshake: u64,
    last_reassert_at: Instant,
    recovery_from_relay: bool,
    endpoint_applied: bool,
}

#[derive(Debug, Clone)]
struct ActiveRelay {
    reservation: RelayReservation,
    next_refresh_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteFailureDisposition {
    Ignore,
    PromoteDirect,
    FallbackToRelay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathMode {
    DirectCandidateKnown,
    DirectProbing,
    DirectActive,
    RelayActive,
    DirectRecoveryProbing,
}

impl Default for PathMode {
    fn default() -> Self {
        Self::DirectCandidateKnown
    }
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
    mut cached_candidates: Vec<Candidate>,
    initial_punch_socket: Option<PunchSocket>,
    mut observed_rx: Option<mpsc::Receiver<ObservedRemoteEndpoint>>,
) {
    let punch_socket = match open_punch_socket(&config, initial_punch_socket).await {
        Ok(socket) => socket,
        Err(err) => {
            warn!("unable to start punch socket runtime: {err:#}");
            return;
        }
    };
    let mut first_connect = true;
    loop {
        if !first_connect || cached_candidates.is_empty() {
            cached_candidates =
                refresh_local_candidates(&config, &punch_socket, &cached_candidates).await;
        }
        first_connect = false;
        match connect_and_run(
            &config,
            &device_id,
            &signal_addr,
            punch_socket.clone(),
            snapshot_rx.clone(),
            update_tx.clone(),
            cached_candidates.clone(),
            &mut observed_rx,
        )
        .await
        {
            Ok(()) => {
                warn!("signal stream closed; reconnecting");
                sleep(Duration::from_secs(2)).await;
            }
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
    punch_socket: Arc<Mutex<PunchSocket>>,
    mut snapshot_rx: watch::Receiver<PeerSnapshot>,
    update_tx: mpsc::Sender<SignalUpdate>,
    mut local_candidates: Vec<Candidate>,
    observed_rx: &mut Option<mpsc::Receiver<ObservedRemoteEndpoint>>,
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

    info!(
        candidates = local_candidates.len(),
        "local punch candidates collected"
    );
    let mut announce_tick = interval(Duration::from_secs(5));
    let mut heartbeat_tick = interval(Duration::from_secs(5));
    let mut punch_tick = interval(Duration::from_millis(500));
    let mut relay_tick = interval(Duration::from_secs(1));
    announce_tick.tick().await;
    heartbeat_tick.tick().await;
    punch_tick.tick().await;
    relay_tick.tick().await;
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
                    local_candidates =
                        refresh_local_candidates(config, &punch_socket, &local_candidates).await;
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
                        bail!("signal stream closed by server");
                    };
                    let snapshot = snapshot_rx.borrow().clone();
                    handle_incoming_envelope(
                        config,
                        device_id,
                        &snapshot,
                        &mut local_candidates,
                        envelope,
                        &outbound_tx,
                        &mut peer_states,
                        &update_tx,
                    ).await?;
                }
                observed = recv_observed_endpoint(observed_rx) => {
                    let Some(observed) = observed else {
                        continue;
                    };
                    let snapshot = snapshot_rx.borrow().clone();
                    handle_observed_remote_endpoint(
                        device_id,
                        &snapshot,
                        &local_candidates,
                        &mut peer_states,
                        &update_tx,
                        &outbound_tx,
                        observed,
                    ).await?;
                }
            }
        }
    }
    .await;

    cleanup_relay_sessions(config, device_id, &mut peer_states).await;
    result
}

pub async fn bootstrap_local_candidates(config: &AgentConfig) -> Vec<Candidate> {
    let mut candidates = baseline_local_candidates(config);
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
            Err(err) => warn!("bootstrap stun query failed: {err:#}"),
        }
    }

    dedupe_candidates(sort_candidates(candidates))
}

pub async fn bootstrap_local_candidates_with_socket(
    config: &AgentConfig,
    punch_socket: &mut PunchSocket,
) -> Vec<Candidate> {
    let mut candidates = baseline_local_candidates(config);
    if let Some(stun_addr) = config.resolved_stun_addr() {
        match punch_socket
            .query_stun(&stun_addr, Duration::from_secs(2))
            .await
        {
            Ok(mapping) => candidates.push(Candidate {
                r#type: CandidateType::PublicIpv4 as i32,
                address: mapping.address,
                port: u32::from(mapping.port),
                network_interface: "punch-socket-stun".to_string(),
                priority: 100,
            }),
            Err(err) => warn!("bootstrap stun query on provided socket failed: {err:#}"),
        }
    }

    dedupe_candidates(sort_candidates(candidates))
}

async fn handle_incoming_envelope(
    config: &AgentConfig,
    device_id: &str,
    snapshot: &PeerSnapshot,
    local_candidates: &mut Vec<Candidate>,
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
            mark_direct_candidate_known(state, "candidate_announcement");
            if should_initiate(device_id, &peer.peer_id) && !direct_path_active_without_relay(state)
            {
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
            mark_direct_candidate_known(state, "punch_request");
            start_attempt_for_peer(config, device_id, peer, state, update_tx, local_candidates)
                .await?;
        }
        Some(Body::PunchResult(body)) => {
            info!(
                peer_id = %peer.peer_id,
                success = body.success,
                reason = %body.reason,
                observed_candidate = body.observed_candidate.is_some(),
                "received punch result"
            );
            let observed_candidate = body.observed_candidate.clone();
            if let Some(candidate) = observed_candidate {
                if observed_local_candidate_matches_remote_lan(&candidate, &state.remote_candidates)
                {
                    info!(
                        peer_id = %peer.peer_id,
                        endpoint = %render_candidate_endpoint(&candidate),
                        "ignoring peer-observed local endpoint that matches peer LAN"
                    );
                } else if update_local_observed_candidate(local_candidates, &candidate) {
                    state.last_observed_remote_candidate = Some(candidate.clone());
                    info!(
                        peer_id = %peer.peer_id,
                        endpoint = %render_candidate_endpoint(&candidate),
                        "updated local public candidate from peer observed source endpoint"
                    );
                    announce_candidates(outbound_tx, device_id, snapshot, local_candidates).await?;
                }
            }
            if body.success {
                if can_accept_remote_direct_progress(state, body.selected_candidate.as_ref()) {
                    if let Some(candidate) = candidate_for_remote_direct_progress(state) {
                        info!(
                            peer_id = %peer.peer_id,
                            endpoint = %render_candidate_endpoint(&candidate),
                            "remote direct progress confirmed; clearing local punch attempt"
                        );
                        state.attempt = None;
                        state.next_direct_retry_at =
                            Some(Instant::now() + direct_retry_delay(config.punch_timeout));
                        state.last_direct_confirmed_at = Some(Instant::now());
                        transition_path_mode(
                            state,
                            PathMode::DirectActive,
                            "remote_direct_progress",
                        );
                        let endpoint_override = candidate_to_endpoint(&candidate);
                        state.last_applied_endpoint = endpoint_override.clone();
                        update_tx
                            .send(SignalUpdate {
                                peer_id: peer.peer_id.clone(),
                                endpoint_override,
                                probe_overlay_ipv4: Some(peer.overlay_ipv4.clone()),
                                reason: "remote_direct_progress".to_string(),
                            })
                            .await
                            .context("apply remote direct progress")?;
                        if let Some(active_relay) = state.relay.take() {
                            info!(peer_id = %peer.peer_id, "direct path recovered; releasing relay");
                            release_active_relay(
                                config,
                                device_id,
                                &peer.peer_id,
                                active_relay,
                                "remote_direct_recovered",
                            )
                            .await;
                        }
                    }
                } else {
                    info!(
                        peer_id = %peer.peer_id,
                        relay_active = state.relay.is_some(),
                        "ignoring remote direct progress while local relay path is still stable"
                    );
                }
            } else if body.observed_candidate.is_none() {
                let latest =
                    latest_handshake(&config.interface_name, &peer.public_key).unwrap_or_default();
                match remote_failure_disposition(state, latest, body.selected_candidate.as_ref()) {
                    RemoteFailureDisposition::Ignore => {
                        info!(
                            peer_id = %peer.peer_id,
                            latest_handshake = latest,
                            relay_active = state.relay.is_some(),
                            "ignoring remote punch failure without active local attempt"
                        );
                    }
                    RemoteFailureDisposition::PromoteDirect => {
                        info!(
                            peer_id = %peer.peer_id,
                            latest_handshake = latest,
                            "ignoring remote punch failure because direct handshake is already observed locally"
                        );
                        complete_direct_attempt(
                            config,
                            device_id,
                            peer,
                            state,
                            outbound_tx,
                            update_tx,
                            "remote_failure_after_local_handshake",
                        )
                        .await?;
                    }
                    RemoteFailureDisposition::FallbackToRelay => {
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
        if state.attempt.is_some() {
            continue;
        }
        if direct_path_active_without_relay(state) {
            continue;
        }
        if !relay_retry_due(state) {
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
    if !relay_retry_due(state) {
        return Ok(());
    }
    if direct_path_active_without_relay(state) {
        return Ok(());
    }

    let latest = latest_handshake(&config.interface_name, &peer.public_key).unwrap_or_default();
    if direct_attempt_suppressed_by_recent_handshake(
        state,
        latest,
        current_unix_timestamp(),
        config.punch_timeout,
    ) {
        return Ok(());
    }

    let Some(candidate) =
        select_stable_remote_candidate(&state.remote_candidates, local_candidates)
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

    let recovery_from_relay = state.relay.is_some();
    let endpoint_applied = !recovery_from_relay
        || direct_recovery_endpoint_apply_allowed(
            state,
            latest,
            current_unix_timestamp(),
            config.punch_timeout,
        );
    let generation = next_attempt_generation(state);
    state.attempt = Some(PunchAttempt {
        generation,
        selected_candidate: candidate.clone(),
        started_at: Instant::now(),
        baseline_handshake: latest,
        last_reassert_at: Instant::now(),
        recovery_from_relay,
        endpoint_applied,
    });
    state.last_selected_candidate = Some(candidate);
    if endpoint_applied {
        state.last_applied_endpoint = Some(endpoint.clone());
    }
    transition_path_mode(
        state,
        if recovery_from_relay {
            PathMode::DirectRecoveryProbing
        } else {
            PathMode::DirectProbing
        },
        if recovery_from_relay {
            "direct_recovery_probe_started"
        } else {
            "punch_started"
        },
    );
    info!(
        peer_id = %peer.peer_id,
        endpoint = %endpoint.render(),
        generation,
        recovery_from_relay,
        endpoint_applied,
        "starting punch attempt"
    );

    if endpoint_applied {
        update_tx
            .send(SignalUpdate {
                peer_id: peer.peer_id.clone(),
                endpoint_override: Some(endpoint),
                probe_overlay_ipv4: Some(peer.overlay_ipv4.clone()),
                reason: if recovery_from_relay {
                    "direct_recovery_probe_started".to_string()
                } else {
                    "punch_started".to_string()
                },
            })
            .await
            .context("send endpoint override update")?;
    }

    Ok(())
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
        if direct_attempt_locally_confirmed(&config.interface_name, peer, &attempt, latest) {
            complete_direct_attempt(
                config,
                device_id,
                peer,
                state,
                outbound_tx,
                update_tx,
                "handshake_observed",
            )
            .await?;
            continue;
        }

        if let Some(observed_candidate) =
            observed_remote_candidate(config, peer, &attempt.selected_candidate)?
        {
            apply_observed_remote_candidate(
                device_id,
                peer,
                state,
                update_tx,
                outbound_tx,
                observed_candidate,
                "wireguard_runtime_observed_candidate",
            )
            .await?;
            continue;
        }

        if attempt.endpoint_applied && attempt.last_reassert_at.elapsed() >= Duration::from_secs(2)
        {
            update_tx
                .send(SignalUpdate {
                    peer_id: peer.peer_id.clone(),
                    endpoint_override: candidate_to_endpoint(&attempt.selected_candidate),
                    probe_overlay_ipv4: Some(peer.overlay_ipv4.clone()),
                    reason: "punch_rearm_set".to_string(),
                })
                .await
                .context("reassert active punch endpoint override")?;
            if let Some(active_attempt) = state.attempt.as_mut() {
                if active_attempt.generation == attempt.generation {
                    active_attempt.last_reassert_at = Instant::now();
                    state.last_applied_endpoint =
                        candidate_to_endpoint(&attempt.selected_candidate);
                }
            }
        }

        probe_overlay_peer(&peer.overlay_ipv4);

        if attempt.started_at.elapsed() >= config.punch_timeout {
            warn!(
                peer_id = %peer.peer_id,
                generation = attempt.generation,
                recovery_from_relay = attempt.recovery_from_relay,
                "hole punch attempt timed out"
            );
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
                        observed_candidate: None,
                    })),
                })
                .await
                .context("send punch failure")?;
            if state
                .attempt
                .as_ref()
                .map(|active| active.generation == attempt.generation)
                .unwrap_or(false)
            {
                state.attempt = None;
                if attempt.endpoint_applied {
                    if is_lan_candidate(&attempt.selected_candidate) {
                        transition_path_mode(
                            state,
                            PathMode::DirectCandidateKnown,
                            "lan_direct_timeout",
                        );
                    } else {
                        fallback_to_relay(
                            config,
                            device_id,
                            peer,
                            state,
                            update_tx,
                            "punch_timeout",
                        )
                        .await?;
                    }
                } else if state.relay.is_some() {
                    transition_path_mode(state, PathMode::RelayActive, "passive_recovery_timeout");
                }
            } else {
                info!(
                    peer_id = %peer.peer_id,
                    generation = attempt.generation,
                    "ignoring stale punch timeout for superseded attempt"
                );
            }
        }
    }
    Ok(())
}

async fn handle_observed_remote_endpoint(
    device_id: &str,
    snapshot: &PeerSnapshot,
    local_candidates: &[Candidate],
    peer_states: &mut BTreeMap<String, PeerSignalState>,
    update_tx: &mpsc::Sender<SignalUpdate>,
    outbound_tx: &mpsc::Sender<SignalEnvelope>,
    observed: ObservedRemoteEndpoint,
) -> Result<()> {
    let Some(peer) = snapshot
        .peers
        .iter()
        .find(|peer| peer.peer_id == observed.peer_id)
    else {
        return Ok(());
    };
    let Some(state) = peer_states.get_mut(&observed.peer_id) else {
        return Ok(());
    };
    let Some(attempt) = state.attempt.as_ref() else {
        return Ok(());
    };

    if observed_remote_endpoint_matches_local_lan(&observed.endpoint, local_candidates) {
        info!(
            peer_id = %peer.peer_id,
            endpoint = %observed.endpoint.render(),
            "ignoring observed remote endpoint that matches local LAN"
        );
        return Ok(());
    }
    if reachable_lan_candidate_exists(&state.remote_candidates, local_candidates) {
        info!(
            peer_id = %peer.peer_id,
            endpoint = %observed.endpoint.render(),
            "ignoring observed remote endpoint because a same-LAN candidate is reachable"
        );
        return Ok(());
    }

    let observed_candidate = Candidate {
        r#type: attempt.selected_candidate.r#type,
        address: observed.endpoint.host,
        port: u32::from(observed.endpoint.port),
        network_interface: "observed".to_string(),
        priority: attempt.selected_candidate.priority.saturating_add(50),
    };
    apply_observed_remote_candidate(
        device_id,
        peer,
        state,
        update_tx,
        outbound_tx,
        observed_candidate,
        "proxy_observed_remote_candidate",
    )
    .await
}

async fn apply_observed_remote_candidate(
    device_id: &str,
    peer: &CachedPeer,
    state: &mut PeerSignalState,
    update_tx: &mpsc::Sender<SignalUpdate>,
    outbound_tx: &mpsc::Sender<SignalEnvelope>,
    observed_candidate: Candidate,
    reason: &str,
) -> Result<()> {
    let changed =
        update_remote_observed_candidate(&mut state.remote_candidates, &observed_candidate);
    let should_report = state
        .last_reported_observed_remote_candidate
        .as_ref()
        .map(|candidate| candidate != &observed_candidate)
        .unwrap_or(true);

    state.last_observed_remote_candidate = Some(observed_candidate.clone());
    state.last_selected_candidate = Some(observed_candidate.clone());
    if let Some(active_attempt) = state.attempt.as_mut() {
        active_attempt.selected_candidate = observed_candidate.clone();
    }

    info!(
        peer_id = %peer.peer_id,
        changed,
        endpoint = %render_candidate_endpoint(&observed_candidate),
        reason,
        "observed remote source endpoint"
    );

    if changed {
        update_tx
            .send(SignalUpdate {
                peer_id: peer.peer_id.clone(),
                endpoint_override: candidate_to_endpoint(&observed_candidate),
                probe_overlay_ipv4: Some(peer.overlay_ipv4.clone()),
                reason: "observed_remote_candidate".to_string(),
            })
            .await
            .context("apply observed remote candidate")?;
        state.last_applied_endpoint = candidate_to_endpoint(&observed_candidate);
    }

    if should_report {
        state.last_reported_observed_remote_candidate = Some(observed_candidate.clone());
        outbound_tx
            .send(SignalEnvelope {
                kind: SignalKind::PunchResult as i32,
                source_device_id: device_id.to_string(),
                target_device_id: peer.peer_id.clone(),
                session_id: session_id(device_id, &peer.peer_id),
                body: Some(Body::PunchResult(PunchResult {
                    success: false,
                    selected_candidate: Some(observed_candidate.clone()),
                    path_type: PathType::HolepunchedIpv4 as i32,
                    reason: reason.to_string(),
                    observed_candidate: Some(observed_candidate),
                })),
            })
            .await
            .context("send observed remote candidate")?;
    }

    Ok(())
}

async fn complete_direct_attempt(
    config: &AgentConfig,
    device_id: &str,
    peer: &CachedPeer,
    state: &mut PeerSignalState,
    outbound_tx: &mpsc::Sender<SignalEnvelope>,
    update_tx: &mpsc::Sender<SignalUpdate>,
    reason: &str,
) -> Result<()> {
    let Some(attempt) = state.attempt.clone() else {
        return Ok(());
    };

    info!(peer_id = %peer.peer_id, reason, "hole punch handshake observed");
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
                reason: reason.to_string(),
                observed_candidate: state.last_observed_remote_candidate.clone(),
            })),
        })
        .await
        .context("send punch success")?;
    state.attempt = None;
    transition_path_mode(state, PathMode::DirectActive, reason);
    let endpoint_override = candidate_to_endpoint(&attempt.selected_candidate);
    state.last_applied_endpoint = endpoint_override.clone();
    update_tx
        .send(SignalUpdate {
            peer_id: peer.peer_id.clone(),
            endpoint_override,
            probe_overlay_ipv4: Some(peer.overlay_ipv4.clone()),
            reason: "direct_path_confirmed".to_string(),
        })
        .await
        .context("apply direct endpoint after local handshake")?;
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
    state.next_direct_retry_at = Some(Instant::now() + direct_retry_delay(config.punch_timeout));
    state.last_direct_confirmed_at = Some(Instant::now());
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

    if endpoint_override.is_some() {
        state.next_direct_retry_at =
            Some(Instant::now() + direct_retry_delay(config.punch_timeout));
        state.last_applied_endpoint = endpoint_override.clone();
        transition_path_mode(state, PathMode::RelayActive, reason);
    }

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

async fn refresh_local_candidates(
    config: &AgentConfig,
    punch_socket: &Arc<Mutex<PunchSocket>>,
    cached_candidates: &[Candidate],
) -> Vec<Candidate> {
    let mut candidates = baseline_local_candidates(config);

    if let Some(stun_addr) = config.resolved_stun_addr() {
        let (state, mapping) = {
            let mut punch_socket = punch_socket.lock().await;
            let state = punch_socket.state();
            if state.shared_receive_runtime {
                (state, None)
            } else {
                (
                    state,
                    Some(
                        punch_socket
                            .query_stun(&stun_addr, Duration::from_secs(2))
                            .await,
                    ),
                )
            }
        };
        match mapping {
            None => {
                let cached_socket_candidates = cached_candidates
                    .iter()
                    .filter(|candidate| {
                        candidate.r#type == CandidateType::PublicIpv4 as i32
                            && candidate.network_interface == "punch-socket-stun"
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                if !cached_socket_candidates.is_empty() {
                    candidates.extend(cached_socket_candidates);
                }
            }
            Some(Ok(mapping)) if state.port_aligned => candidates.push(Candidate {
                r#type: CandidateType::PublicIpv4 as i32,
                address: mapping.address,
                port: u32::from(mapping.port),
                network_interface: "punch-socket-stun".to_string(),
                priority: 50,
            }),
            Some(Ok(mapping)) => {
                info!(
                    endpoint = %Endpoint {
                        host: mapping.address,
                        port: mapping.port,
                    }.render(),
                    "ignoring non-aligned punch socket mapping for advertised candidates"
                );
            }
            Some(Err(err)) => {
                if !state.port_aligned {
                    warn!("stun query failed on non-aligned punch socket: {err:#}");
                } else {
                    let cached_socket_candidates = cached_candidates
                        .iter()
                        .filter(|candidate| {
                            candidate.r#type == CandidateType::PublicIpv4 as i32
                                && candidate.network_interface == "punch-socket-stun"
                        })
                        .cloned()
                        .collect::<Vec<_>>();

                    if cached_socket_candidates.is_empty() {
                        warn!("stun query failed: {err:#}");
                    } else {
                        info!(
                            candidates = cached_socket_candidates.len(),
                            "reusing cached stun candidates after query failure"
                        );
                        candidates.extend(cached_socket_candidates);
                    }
                }
            }
        }
    }

    candidates.extend(
        cached_candidates
            .iter()
            .filter(|candidate| {
                candidate.r#type == CandidateType::PublicIpv4 as i32
                    && (candidate.network_interface == "stun"
                        || candidate.network_interface == "observed")
            })
            .cloned(),
    );

    dedupe_candidates(sort_candidates(candidates))
}

fn baseline_local_candidates(config: &AgentConfig) -> Vec<Candidate> {
    let listen_port = u32::from(config.listen_port.unwrap_or_default());
    if listen_port == 0 {
        return Vec::new();
    }

    let mut candidates = Vec::new();
    for (address, interface_name) in collect_lan_ipv4s().unwrap_or_default() {
        if interface_name == config.interface_name {
            continue;
        }
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

    candidates
}

async fn open_punch_socket(
    config: &AgentConfig,
    initial_punch_socket: Option<PunchSocket>,
) -> Result<Arc<Mutex<PunchSocket>>> {
    let punch_socket = match initial_punch_socket {
        Some(punch_socket) => punch_socket,
        None => {
            #[cfg(windows)]
            {
                PunchSocket::bind_with_requested_port(None, config.listen_port).await?
            }
            #[cfg(not(windows))]
            {
                PunchSocket::bind(config.listen_port).await?
            }
        }
    };
    let state = punch_socket.state();
    info!(
        requested_port = state.requested_port.unwrap_or_default(),
        bound_addr = %state.local_addr,
        port_aligned = state.port_aligned,
        "punch socket runtime ready"
    );
    Ok(Arc::new(Mutex::new(punch_socket)))
}

async fn recv_observed_endpoint(
    observed_rx: &mut Option<mpsc::Receiver<ObservedRemoteEndpoint>>,
) -> Option<ObservedRemoteEndpoint> {
    match observed_rx.as_mut() {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

fn candidate_to_endpoint(candidate: &Candidate) -> Option<Endpoint> {
    u16::try_from(candidate.port).ok().map(|port| Endpoint {
        host: candidate.address.clone(),
        port,
    })
}

fn update_remote_observed_candidate(
    remote_candidates: &mut Vec<Candidate>,
    observed: &Candidate,
) -> bool {
    let mut changed = false;
    remote_candidates.retain(|candidate| {
        let replace_candidate = candidate.r#type == observed.r#type
            && (candidate.address == observed.address
                || candidate.network_interface == "stun"
                || candidate.network_interface == "static"
                || candidate.network_interface == "punch-socket-stun"
                || candidate.network_interface == "observed");
        if replace_candidate {
            changed = true;
            return false;
        }
        true
    });
    remote_candidates.push(observed.clone());
    *remote_candidates = dedupe_candidates(sort_candidates(std::mem::take(remote_candidates)));
    changed
        || remote_candidates
            .iter()
            .any(|candidate| candidate == observed)
}

fn observed_remote_candidate(
    config: &AgentConfig,
    peer: &CachedPeer,
    selected_candidate: &Candidate,
) -> Result<Option<Candidate>> {
    let Some(endpoint) = current_peer_endpoint(&config.interface_name, &peer.public_key)? else {
        return Ok(None);
    };

    let is_same_public_ip = matches!(
        CandidateType::try_from(selected_candidate.r#type).ok(),
        Some(CandidateType::PublicIpv4 | CandidateType::PublicIpv6)
    ) && endpoint.host == selected_candidate.address
        && endpoint.port != selected_candidate.port as u16;

    if !is_same_public_ip {
        return Ok(None);
    }

    Ok(Some(Candidate {
        r#type: selected_candidate.r#type,
        address: endpoint.host,
        port: u32::from(endpoint.port),
        network_interface: "observed".to_string(),
        priority: selected_candidate.priority.saturating_add(50),
    }))
}

fn update_local_observed_candidate(
    local_candidates: &mut Vec<Candidate>,
    observed: &Candidate,
) -> bool {
    let Some(observed_type) = CandidateType::try_from(observed.r#type).ok() else {
        return false;
    };
    if !matches!(
        observed_type,
        CandidateType::PublicIpv4 | CandidateType::PublicIpv6
    ) {
        return false;
    }

    let mut changed = false;
    local_candidates.retain(|candidate| {
        let same_public_family = candidate.r#type == observed.r#type;
        let replace_candidate = same_public_family
            && (candidate.address == observed.address
                || candidate.network_interface == "punch-socket-stun"
                || candidate.network_interface == "punch-socket-observed"
                || candidate.network_interface == "observed"
                || candidate.network_interface == "static");
        if replace_candidate {
            changed = true;
            return false;
        }
        true
    });

    local_candidates.push(observed.clone());
    *local_candidates = dedupe_candidates(sort_candidates(std::mem::take(local_candidates)));
    changed
        || local_candidates
            .iter()
            .any(|candidate| candidate == observed)
}

fn observed_remote_endpoint_matches_local_lan(
    observed: &Endpoint,
    local_candidates: &[Candidate],
) -> bool {
    local_candidates.iter().any(|candidate| {
        is_lan_candidate(candidate) && same_ipv4_subnet_24(&observed.host, &candidate.address)
    })
}

fn observed_local_candidate_matches_remote_lan(
    observed: &Candidate,
    remote_candidates: &[Candidate],
) -> bool {
    if !matches!(
        CandidateType::try_from(observed.r#type).ok(),
        Some(CandidateType::PublicIpv4)
    ) {
        return false;
    }

    remote_candidates.iter().any(|candidate| {
        is_lan_candidate(candidate) && same_ipv4_subnet_24(&observed.address, &candidate.address)
    })
}

fn is_lan_candidate(candidate: &Candidate) -> bool {
    matches!(
        CandidateType::try_from(candidate.r#type).ok(),
        Some(CandidateType::Lan)
    )
}

fn same_ipv4_subnet_24(left: &str, right: &str) -> bool {
    let Ok(left) = left.parse::<std::net::Ipv4Addr>() else {
        return false;
    };
    let Ok(right) = right.parse::<std::net::Ipv4Addr>() else {
        return false;
    };

    let left = left.octets();
    let right = right.octets();
    left[0..3] == right[0..3]
}

fn render_candidate_endpoint(candidate: &Candidate) -> String {
    let port = u16::try_from(candidate.port).unwrap_or_default();
    Endpoint {
        host: candidate.address.clone(),
        port,
    }
    .render()
}

fn relay_retry_due(state: &PeerSignalState) -> bool {
    match state.next_direct_retry_at {
        Some(next_retry_at) => Instant::now() >= next_retry_at,
        None => true,
    }
}

fn next_attempt_generation(state: &mut PeerSignalState) -> u64 {
    state.attempt_generation = state.attempt_generation.saturating_add(1).max(1);
    state.attempt_generation
}

fn transition_path_mode(state: &mut PeerSignalState, mode: PathMode, reason: &str) {
    if state.path_mode != mode {
        state.path_mode = mode;
        state.last_transition_at = Some(Instant::now());
    }
    state.last_transition_reason = Some(reason.to_string());
}

fn mark_direct_candidate_known(state: &mut PeerSignalState, reason: &str) {
    if direct_path_active_without_relay(state) {
        state.last_transition_reason = Some(reason.to_string());
        return;
    }
    transition_path_mode(state, PathMode::DirectCandidateKnown, reason);
}

fn remote_failure_disposition(
    state: &PeerSignalState,
    latest_handshake: u64,
    selected_candidate: Option<&Candidate>,
) -> RemoteFailureDisposition {
    match state.attempt.as_ref() {
        None if recent_direct_confirmation(state) => RemoteFailureDisposition::Ignore,
        None if state.relay.is_none() => RemoteFailureDisposition::FallbackToRelay,
        None => RemoteFailureDisposition::Ignore,
        Some(attempt)
            if selected_candidate
                .map(|candidate| !same_candidate(candidate, &attempt.selected_candidate))
                .unwrap_or(false) =>
        {
            RemoteFailureDisposition::Ignore
        }
        Some(attempt) if is_lan_candidate(&attempt.selected_candidate) => {
            RemoteFailureDisposition::Ignore
        }
        Some(attempt) if attempt.recovery_from_relay && !attempt.endpoint_applied => {
            RemoteFailureDisposition::Ignore
        }
        Some(attempt) if latest_handshake > attempt.baseline_handshake && latest_handshake > 0 => {
            RemoteFailureDisposition::PromoteDirect
        }
        Some(_) => RemoteFailureDisposition::FallbackToRelay,
    }
}

fn recent_direct_confirmation(state: &PeerSignalState) -> bool {
    state
        .last_direct_confirmed_at
        .map(|confirmed_at| confirmed_at.elapsed() <= Duration::from_secs(10))
        .unwrap_or(false)
}

fn direct_retry_delay(punch_timeout: Duration) -> Duration {
    Duration::from_secs((punch_timeout.as_secs().max(3) * 3).max(direct_retry_min_secs()))
}

fn direct_retry_min_secs() -> u64 {
    60
}

fn direct_health_window(punch_timeout: Duration) -> Duration {
    Duration::from_secs((punch_timeout.as_secs().max(3) * 2).max(direct_retry_min_secs()))
}

fn direct_attempt_suppressed_by_recent_handshake(
    state: &PeerSignalState,
    latest_handshake: u64,
    now_unix: u64,
    punch_timeout: Duration,
) -> bool {
    if state.relay.is_some() || latest_handshake == 0 {
        return false;
    }

    now_unix.saturating_sub(latest_handshake) <= direct_health_window(punch_timeout).as_secs()
}

fn direct_recovery_endpoint_apply_allowed(
    state: &PeerSignalState,
    latest_handshake: u64,
    now_unix: u64,
    punch_timeout: Duration,
) -> bool {
    state.relay.is_some()
        && latest_handshake > 0
        && now_unix.saturating_sub(latest_handshake)
            <= direct_health_window(punch_timeout).as_secs()
}

fn direct_path_active_without_relay(state: &PeerSignalState) -> bool {
    state.relay.is_none() && matches!(state.path_mode, PathMode::DirectActive)
}

fn direct_attempt_locally_confirmed(
    interface_name: &str,
    peer: &CachedPeer,
    attempt: &PunchAttempt,
    latest_handshake: u64,
) -> bool {
    if latest_handshake <= attempt.baseline_handshake || latest_handshake == 0 {
        return false;
    }
    if !attempt.recovery_from_relay || attempt.endpoint_applied {
        return true;
    }

    current_peer_endpoint(interface_name, &peer.public_key)
        .ok()
        .flatten()
        .map(|endpoint| endpoint_matches_candidate(&endpoint, &attempt.selected_candidate))
        .unwrap_or(false)
}

fn candidate_for_remote_direct_progress(state: &PeerSignalState) -> Option<Candidate> {
    state
        .last_observed_remote_candidate
        .clone()
        .or_else(|| {
            state
                .attempt
                .as_ref()
                .map(|attempt| attempt.selected_candidate.clone())
        })
        .or_else(|| state.last_selected_candidate.clone())
}

fn can_accept_remote_direct_progress(
    state: &PeerSignalState,
    selected_candidate: Option<&Candidate>,
) -> bool {
    if state.relay.is_none() {
        return true;
    }

    let Some(attempt) = state.attempt.as_ref() else {
        return false;
    };
    attempt.endpoint_applied
        || selected_candidate
            .map(|candidate| same_candidate(candidate, &attempt.selected_candidate))
            .unwrap_or(false)
}

fn select_stable_remote_candidate(
    remote_candidates: &[Candidate],
    local_candidates: &[Candidate],
) -> Option<Candidate> {
    if let Some(lan) = select_remote_candidate_for_local(
        &remote_candidates
            .iter()
            .filter(|candidate| is_lan_candidate(candidate))
            .cloned()
            .collect::<Vec<_>>(),
        local_candidates,
    )
    .filter(|candidate| lan_candidate_reachable_locally(candidate, local_candidates))
    {
        return Some(lan);
    }

    select_remote_candidate_for_local(remote_candidates, local_candidates)
}

fn reachable_lan_candidate_exists(
    remote_candidates: &[Candidate],
    local_candidates: &[Candidate],
) -> bool {
    remote_candidates.iter().any(|candidate| {
        is_lan_candidate(candidate) && lan_candidate_reachable_locally(candidate, local_candidates)
    })
}

fn lan_candidate_reachable_locally(candidate: &Candidate, local_candidates: &[Candidate]) -> bool {
    local_candidates.iter().any(|local| {
        is_lan_candidate(local) && same_ipv4_subnet_24(&candidate.address, &local.address)
    })
}

fn same_candidate(left: &Candidate, right: &Candidate) -> bool {
    left.r#type == right.r#type && left.address == right.address && left.port == right.port
}

fn endpoint_matches_candidate(endpoint: &Endpoint, candidate: &Candidate) -> bool {
    endpoint.host == candidate.address && u32::from(endpoint.port) == candidate.port
}

fn dedupe_candidates(candidates: Vec<Candidate>) -> Vec<Candidate> {
    let mut positions = BTreeMap::new();
    let mut deduped = Vec::new();
    for candidate in candidates {
        let key = (candidate.r#type, candidate.address.clone(), candidate.port);
        match positions.get(&key).copied() {
            None => {
                positions.insert(key, deduped.len());
                deduped.push(candidate);
            }
            Some(index)
                if candidate_source_rank(&candidate) > candidate_source_rank(&deduped[index]) =>
            {
                deduped[index] = candidate;
            }
            Some(_) => {}
        }
    }
    deduped
}

fn candidate_source_rank(candidate: &Candidate) -> u8 {
    match candidate.network_interface.as_str() {
        "observed" => 3,
        "stun" => 2,
        "punch-socket-stun" => 1,
        _ => 0,
    }
}

fn collect_lan_ipv4s() -> Result<Vec<(String, String)>> {
    #[cfg(windows)]
    {
        return collect_windows_lan_ipv4s();
    }

    #[cfg(not(windows))]
    {
        return collect_unix_lan_ipv4s();
    }
}

#[cfg(not(windows))]
fn collect_unix_lan_ipv4s() -> Result<Vec<(String, String)>> {
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

#[cfg(windows)]
fn collect_windows_lan_ipv4s() -> Result<Vec<(String, String)>> {
    let script = concat!(
        "$configs = Get-NetIPConfiguration | ForEach-Object {",
        "  $alias = $_.InterfaceAlias;",
        "  foreach ($addr in $_.IPv4Address) {",
        "    [PSCustomObject]@{IPAddress = $addr.IPAddress; InterfaceAlias = $alias}",
        "  }",
        "};",
        "$configs | ConvertTo-Json -Compress"
    );
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .output()
        .context("run powershell for candidate collection")?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    parse_windows_interface_json(&String::from_utf8_lossy(&output.stdout))
}

#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Deserialize)]
struct WindowsLanAddress {
    #[serde(rename = "IPAddress")]
    ip_address: String,
    #[serde(rename = "InterfaceAlias")]
    interface_alias: String,
}

#[cfg_attr(not(windows), allow(dead_code))]
fn parse_windows_interface_json(output: &str) -> Result<Vec<(String, String)>> {
    let trimmed = output.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return Ok(Vec::new());
    }

    let addresses = if trimmed.starts_with('[') {
        serde_json::from_str::<Vec<WindowsLanAddress>>(trimmed)
            .context("parse windows candidate array")?
    } else {
        vec![serde_json::from_str::<WindowsLanAddress>(trimmed)
            .context("parse windows candidate object")?]
    };

    Ok(addresses
        .into_iter()
        .filter_map(|address| {
            let ip = address.ip_address.trim();
            if ip.is_empty() || ip.starts_with("127.") || ip.starts_with("169.254.") {
                return None;
            }
            Some((ip.to_string(), address.interface_alias))
        })
        .collect())
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

fn current_unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
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

    let ping_bin = if cfg!(windows) {
        "ping"
    } else if std::path::Path::new("/usr/bin/ping").exists() {
        "/usr/bin/ping"
    } else {
        "ping"
    };

    #[cfg(windows)]
    let args = ["-n", "5", "-w", "1000", overlay_ipv4];
    #[cfg(not(windows))]
    let args = ["-c", "5", "-i", "0.2", "-W", "1", overlay_ipv4];

    let _ = Command::new(ping_bin)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use super::{
        can_accept_remote_direct_progress, candidate_for_remote_direct_progress,
        current_unix_timestamp, dedupe_candidates, direct_attempt_suppressed_by_recent_handshake,
        direct_health_window, direct_path_active_without_relay,
        direct_recovery_endpoint_apply_allowed, direct_retry_delay, mark_direct_candidate_known,
        observed_local_candidate_matches_remote_lan, observed_remote_endpoint_matches_local_lan,
        parse_ip_addr_output, parse_windows_interface_json, reachable_lan_candidate_exists,
        relay_host, relay_refresh_delay, remote_failure_disposition,
        select_stable_remote_candidate, ActiveRelay, PathMode, PeerSignalState, PunchAttempt,
        RemoteFailureDisposition,
    };
    use api_client::proto::{Candidate, CandidateType};
    use relay_client::RelayReservation;
    use std::time::{Duration, Instant};
    use wg_manager::Endpoint;

    #[test]
    fn parse_ip_addr_output_extracts_global_ipv4_addresses() {
        let parsed = parse_ip_addr_output(
            "2: eth0    inet 192.0.2.10/24 brd 192.0.2.255 scope global eth0\n3: lo    inet 127.0.0.1/8 scope host lo\n",
        );

        assert_eq!(parsed, vec![("192.0.2.10".to_string(), "eth0".to_string())]);
    }

    #[test]
    fn parse_windows_interface_json_extracts_non_loopback_ipv4_addresses() {
        let parsed = parse_windows_interface_json(
            r#"[{"IPAddress":"10.10.1.20","InterfaceAlias":"Ethernet"},{"IPAddress":"127.0.0.1","InterfaceAlias":"Loopback"}]"#,
        )
        .expect("parse windows interface json");

        assert_eq!(
            parsed,
            vec![("10.10.1.20".to_string(), "Ethernet".to_string())]
        );
    }

    #[test]
    fn parse_windows_interface_json_accepts_single_object() {
        let parsed = parse_windows_interface_json(
            r#"{"IPAddress":"192.168.123.50","InterfaceAlias":"vEthernet"}"#,
        )
        .expect("parse single windows interface json object");

        assert_eq!(
            parsed,
            vec![("192.168.123.50".to_string(), "vEthernet".to_string())]
        );
    }

    #[test]
    fn relay_refresh_delay_uses_half_ttl() {
        assert_eq!(relay_refresh_delay(30), Duration::from_secs(15));
        assert_eq!(relay_refresh_delay(1), Duration::from_secs(1));
    }

    #[test]
    fn direct_retry_delay_has_floor_and_scales_with_timeout() {
        assert_eq!(
            direct_retry_delay(Duration::from_secs(3)),
            Duration::from_secs(60)
        );
        assert_eq!(
            direct_retry_delay(Duration::from_secs(30)),
            Duration::from_secs(90)
        );
    }

    #[test]
    fn direct_health_window_has_floor_and_scales_with_timeout() {
        assert_eq!(
            direct_health_window(Duration::from_secs(5)),
            Duration::from_secs(60)
        );
        assert_eq!(
            direct_health_window(Duration::from_secs(45)),
            Duration::from_secs(90)
        );
    }

    #[test]
    fn direct_attempt_is_suppressed_while_direct_handshake_is_recent() {
        let state = PeerSignalState::default();

        assert!(direct_attempt_suppressed_by_recent_handshake(
            &state,
            100,
            130,
            Duration::from_secs(5)
        ));
    }

    #[test]
    fn direct_attempt_is_allowed_when_relay_is_active() {
        let state = PeerSignalState {
            relay: Some(ActiveRelay {
                reservation: RelayReservation {
                    relay_host: "198.51.100.10".to_string(),
                    udp_port: 45000,
                    ttl_seconds: 30,
                    session_id: "session".to_string(),
                },
                next_refresh_at: Instant::now(),
            }),
            ..PeerSignalState::default()
        };

        assert!(!direct_attempt_suppressed_by_recent_handshake(
            &state,
            100,
            130,
            Duration::from_secs(5)
        ));
    }

    #[test]
    fn direct_active_path_without_relay_suppresses_periodic_reprobe() {
        let state = PeerSignalState {
            path_mode: PathMode::DirectActive,
            ..PeerSignalState::default()
        };

        assert!(direct_path_active_without_relay(&state));
    }

    #[test]
    fn candidate_signal_does_not_demote_direct_active_path() {
        let mut state = PeerSignalState {
            path_mode: PathMode::DirectActive,
            ..PeerSignalState::default()
        };

        mark_direct_candidate_known(&mut state, "candidate_announcement");

        assert_eq!(state.path_mode, PathMode::DirectActive);
        assert_eq!(
            state.last_transition_reason.as_deref(),
            Some("candidate_announcement")
        );
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

    #[test]
    fn dedupe_candidates_prefers_punch_socket_stun_over_static_for_same_public_endpoint() {
        let deduped = dedupe_candidates(vec![
            Candidate {
                r#type: CandidateType::PublicIpv4 as i32,
                address: "198.51.100.10".to_string(),
                port: 51820,
                network_interface: "static".to_string(),
                priority: 200,
            },
            Candidate {
                r#type: CandidateType::PublicIpv4 as i32,
                address: "198.51.100.10".to_string(),
                port: 51820,
                network_interface: "punch-socket-stun".to_string(),
                priority: 100,
            },
        ]);

        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].network_interface, "punch-socket-stun");
    }

    #[test]
    fn dedupe_candidates_keeps_bootstrap_stun_over_punch_socket_stun() {
        let deduped = dedupe_candidates(vec![
            Candidate {
                r#type: CandidateType::PublicIpv4 as i32,
                address: "198.51.100.10".to_string(),
                port: 51820,
                network_interface: "stun".to_string(),
                priority: 100,
            },
            Candidate {
                r#type: CandidateType::PublicIpv4 as i32,
                address: "198.51.100.10".to_string(),
                port: 51820,
                network_interface: "punch-socket-stun".to_string(),
                priority: 50,
            },
        ]);

        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].network_interface, "stun");
    }

    #[test]
    fn remote_failure_without_local_attempt_falls_back_when_no_relay_exists() {
        let state = PeerSignalState::default();

        assert_eq!(
            remote_failure_disposition(&state, 10, None),
            RemoteFailureDisposition::FallbackToRelay
        );
    }

    #[test]
    fn remote_failure_after_recent_direct_confirmation_is_ignored() {
        let state = PeerSignalState {
            last_direct_confirmed_at: Some(Instant::now()),
            ..PeerSignalState::default()
        };

        assert_eq!(
            remote_failure_disposition(&state, 10, None),
            RemoteFailureDisposition::Ignore
        );
    }

    #[test]
    fn remote_failure_without_local_attempt_is_ignored_when_relay_exists() {
        let state = PeerSignalState {
            relay: Some(ActiveRelay {
                reservation: RelayReservation {
                    relay_host: "198.51.100.10".to_string(),
                    udp_port: 45000,
                    ttl_seconds: 30,
                    session_id: "session".to_string(),
                },
                next_refresh_at: Instant::now(),
            }),
            ..PeerSignalState::default()
        };

        assert_eq!(
            remote_failure_disposition(&state, 0, None),
            RemoteFailureDisposition::Ignore
        );
    }

    #[test]
    fn remote_failure_promotes_direct_when_handshake_advanced() {
        let mut state = PeerSignalState::default();
        state.attempt = Some(PunchAttempt {
            generation: 1,
            selected_candidate: Candidate {
                r#type: CandidateType::PublicIpv4 as i32,
                address: "198.51.100.10".to_string(),
                port: 51820,
                network_interface: "stun".to_string(),
                priority: 100,
            },
            started_at: Instant::now(),
            baseline_handshake: 10,
            last_reassert_at: Instant::now(),
            recovery_from_relay: false,
            endpoint_applied: true,
        });

        assert_eq!(
            remote_failure_disposition(&state, 11, None),
            RemoteFailureDisposition::PromoteDirect
        );
    }

    #[test]
    fn remote_failure_falls_back_while_attempt_is_still_pending() {
        let mut state = PeerSignalState::default();
        let selected = Candidate {
            r#type: CandidateType::PublicIpv4 as i32,
            address: "198.51.100.10".to_string(),
            port: 51820,
            network_interface: "stun".to_string(),
            priority: 100,
        };
        state.attempt = Some(PunchAttempt {
            generation: 1,
            selected_candidate: selected,
            started_at: Instant::now(),
            baseline_handshake: 10,
            last_reassert_at: Instant::now(),
            recovery_from_relay: false,
            endpoint_applied: true,
        });

        assert_eq!(
            remote_failure_disposition(&state, 10, None),
            RemoteFailureDisposition::FallbackToRelay
        );
    }

    #[test]
    fn stale_remote_failure_for_different_candidate_is_ignored() {
        let mut state = PeerSignalState::default();
        state.attempt = Some(PunchAttempt {
            generation: 2,
            selected_candidate: Candidate {
                r#type: CandidateType::PublicIpv4 as i32,
                address: "198.51.100.10".to_string(),
                port: 51820,
                network_interface: "stun".to_string(),
                priority: 100,
            },
            started_at: Instant::now(),
            baseline_handshake: 10,
            last_reassert_at: Instant::now(),
            recovery_from_relay: false,
            endpoint_applied: true,
        });

        let stale = Candidate {
            r#type: CandidateType::PublicIpv4 as i32,
            address: "198.51.100.11".to_string(),
            port: 51820,
            network_interface: "stun".to_string(),
            priority: 100,
        };

        assert_eq!(
            remote_failure_disposition(&state, 10, Some(&stale)),
            RemoteFailureDisposition::Ignore
        );
    }

    #[test]
    fn remote_failure_for_lan_candidate_does_not_fallback_to_relay() {
        let mut state = PeerSignalState::default();
        state.attempt = Some(PunchAttempt {
            generation: 1,
            selected_candidate: Candidate {
                r#type: CandidateType::Lan as i32,
                address: "10.10.1.10".to_string(),
                port: 51820,
                network_interface: "lan0".to_string(),
                priority: 300,
            },
            started_at: Instant::now(),
            baseline_handshake: 0,
            last_reassert_at: Instant::now(),
            recovery_from_relay: false,
            endpoint_applied: true,
        });

        assert_eq!(
            remote_failure_disposition(&state, 0, None),
            RemoteFailureDisposition::Ignore
        );
    }

    #[test]
    fn path_mode_tracks_relay_recovery_attempts() {
        let state = PeerSignalState {
            path_mode: PathMode::DirectRecoveryProbing,
            attempt_generation: 3,
            relay: Some(ActiveRelay {
                reservation: RelayReservation {
                    relay_host: "198.51.100.10".to_string(),
                    udp_port: 45000,
                    ttl_seconds: 30,
                    session_id: "session".to_string(),
                },
                next_refresh_at: Instant::now(),
            }),
            ..PeerSignalState::default()
        };

        assert_eq!(state.path_mode, PathMode::DirectRecoveryProbing);
        assert_eq!(state.attempt_generation, 3);
        assert!(state.relay.is_some());
    }

    #[test]
    fn remote_direct_progress_keeps_local_remote_candidate_mapping() {
        let expected = Candidate {
            r#type: CandidateType::PublicIpv4 as i32,
            address: "192.0.2.10".to_string(),
            port: 51820,
            network_interface: "stun".to_string(),
            priority: 100,
        };

        let state = PeerSignalState {
            last_selected_candidate: Some(expected.clone()),
            ..PeerSignalState::default()
        };

        assert_eq!(candidate_for_remote_direct_progress(&state), Some(expected));
    }

    #[test]
    fn remote_direct_progress_prefers_observed_candidate() {
        let observed = Candidate {
            r#type: CandidateType::PublicIpv4 as i32,
            address: "198.51.100.30".to_string(),
            port: 55000,
            network_interface: "observed".to_string(),
            priority: 150,
        };

        let state = PeerSignalState {
            last_selected_candidate: Some(Candidate {
                r#type: CandidateType::PublicIpv4 as i32,
                address: "192.0.2.10".to_string(),
                port: 51820,
                network_interface: "stun".to_string(),
                priority: 100,
            }),
            last_observed_remote_candidate: Some(observed.clone()),
            ..PeerSignalState::default()
        };

        assert_eq!(candidate_for_remote_direct_progress(&state), Some(observed));
    }

    #[test]
    fn relay_recovery_endpoint_apply_requires_recent_relay_handshake() {
        let state = PeerSignalState {
            relay: Some(ActiveRelay {
                reservation: RelayReservation {
                    relay_host: "198.51.100.10".to_string(),
                    udp_port: 45000,
                    ttl_seconds: 30,
                    session_id: "session".to_string(),
                },
                next_refresh_at: Instant::now(),
            }),
            ..PeerSignalState::default()
        };
        let now = current_unix_timestamp();

        assert!(!direct_recovery_endpoint_apply_allowed(
            &state,
            0,
            now,
            Duration::from_secs(5),
        ));
        assert!(direct_recovery_endpoint_apply_allowed(
            &state,
            now,
            now,
            Duration::from_secs(5),
        ));
        assert!(!direct_recovery_endpoint_apply_allowed(
            &state,
            now.saturating_sub(120),
            now,
            Duration::from_secs(5),
        ));
    }

    #[test]
    fn remote_direct_progress_is_ignored_when_relay_attempt_has_not_applied_endpoint() {
        let state = PeerSignalState {
            relay: Some(ActiveRelay {
                reservation: RelayReservation {
                    relay_host: "198.51.100.10".to_string(),
                    udp_port: 45000,
                    ttl_seconds: 30,
                    session_id: "session".to_string(),
                },
                next_refresh_at: Instant::now(),
            }),
            attempt: Some(PunchAttempt {
                generation: 1,
                selected_candidate: Candidate {
                    r#type: CandidateType::PublicIpv4 as i32,
                    address: "198.51.100.20".to_string(),
                    port: 51820,
                    network_interface: "stun".to_string(),
                    priority: 100,
                },
                started_at: Instant::now(),
                baseline_handshake: 0,
                last_reassert_at: Instant::now(),
                recovery_from_relay: true,
                endpoint_applied: false,
            }),
            ..PeerSignalState::default()
        };

        assert!(!can_accept_remote_direct_progress(&state, None));
    }

    #[test]
    fn relay_recovery_accepts_matching_remote_direct_success() {
        let selected = Candidate {
            r#type: CandidateType::PublicIpv4 as i32,
            address: "198.51.100.20".to_string(),
            port: 51820,
            network_interface: "stun".to_string(),
            priority: 100,
        };
        let state = PeerSignalState {
            relay: Some(ActiveRelay {
                reservation: RelayReservation {
                    relay_host: "198.51.100.10".to_string(),
                    udp_port: 45000,
                    ttl_seconds: 30,
                    session_id: "session".to_string(),
                },
                next_refresh_at: Instant::now(),
            }),
            attempt: Some(PunchAttempt {
                generation: 1,
                selected_candidate: selected.clone(),
                started_at: Instant::now(),
                baseline_handshake: 0,
                last_reassert_at: Instant::now(),
                recovery_from_relay: true,
                endpoint_applied: false,
            }),
            ..PeerSignalState::default()
        };

        assert!(can_accept_remote_direct_progress(&state, Some(&selected)));
    }

    #[test]
    fn proxy_observation_rejects_local_lan_gateway_endpoint() {
        let local_candidates = vec![Candidate {
            r#type: CandidateType::Lan as i32,
            address: "10.10.1.10".to_string(),
            port: 51820,
            network_interface: "lan0".to_string(),
            priority: 300,
        }];

        assert!(observed_remote_endpoint_matches_local_lan(
            &Endpoint {
                host: "10.10.1.254".to_string(),
                port: 36653,
            },
            &local_candidates
        ));
        assert!(!observed_remote_endpoint_matches_local_lan(
            &Endpoint {
                host: "192.168.123.221".to_string(),
                port: 51821,
            },
            &local_candidates
        ));
    }

    #[test]
    fn peer_observed_local_candidate_rejects_peer_lan_gateway_endpoint() {
        let remote_candidates = vec![Candidate {
            r#type: CandidateType::Lan as i32,
            address: "10.10.2.10".to_string(),
            port: 51821,
            network_interface: "lan0".to_string(),
            priority: 300,
        }];

        assert!(observed_local_candidate_matches_remote_lan(
            &Candidate {
                r#type: CandidateType::PublicIpv4 as i32,
                address: "10.10.2.254".to_string(),
                port: 51820,
                network_interface: "observed".to_string(),
                priority: 150,
            },
            &remote_candidates
        ));
        assert!(!observed_local_candidate_matches_remote_lan(
            &Candidate {
                r#type: CandidateType::PublicIpv4 as i32,
                address: "192.168.123.211".to_string(),
                port: 51820,
                network_interface: "observed".to_string(),
                priority: 150,
            },
            &remote_candidates
        ));
    }

    #[test]
    fn stable_candidate_selection_prefers_reachable_lan_over_observed_public() {
        let remote_candidates = vec![
            Candidate {
                r#type: CandidateType::PublicIpv4 as i32,
                address: "192.168.123.211".to_string(),
                port: 10934,
                network_interface: "observed".to_string(),
                priority: 500,
            },
            Candidate {
                r#type: CandidateType::Lan as i32,
                address: "10.10.1.20".to_string(),
                port: 51830,
                network_interface: "Ethernet".to_string(),
                priority: 300,
            },
        ];
        let local_candidates = vec![Candidate {
            r#type: CandidateType::Lan as i32,
            address: "10.10.1.10".to_string(),
            port: 51820,
            network_interface: "eth0".to_string(),
            priority: 300,
        }];

        let selected = select_stable_remote_candidate(&remote_candidates, &local_candidates)
            .expect("selected candidate");

        assert_eq!(selected.address, "10.10.1.20");
        assert!(reachable_lan_candidate_exists(
            &remote_candidates,
            &local_candidates
        ));
    }
}
