mod signal;

use std::{
    collections::BTreeMap,
    fs,
    future::pending,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use api_client::proto::management_service_client::ManagementServiceClient;
use api_client::proto::{
    DirectEndpoint, Peer, RegisterDeviceRequest, SyncConfigEvent, SyncConfigEventType,
    SyncConfigRequest,
};
use netlink_linux::LinuxWireGuardBackend;
use serde::{de::Error as DeError, Deserialize, Deserializer};
use tokio::time::sleep;
use tracing::{error, info, warn};
use wg_manager::{build_desired_state_with_overrides, Endpoint, WireGuardBackend};
use wintun_windows::WindowsWireGuardBackend;

#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    pub node_name: String,
    pub management_addr: String,
    #[serde(default)]
    pub signal_addr: Option<String>,
    #[serde(default)]
    pub relay_addr: Option<String>,
    #[serde(default)]
    pub stun_addr: Option<String>,
    pub bootstrap_token: String,
    pub public_key: String,
    pub interface_name: String,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default = "default_os")]
    pub os: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default)]
    pub private_key: Option<String>,
    #[serde(default)]
    pub listen_port: Option<u16>,
    #[serde(default)]
    pub advertise_host: Option<String>,
    #[serde(default)]
    pub advertised_routes: Vec<String>,
    #[serde(
        default = "default_punch_timeout",
        deserialize_with = "deserialize_duration"
    )]
    pub punch_timeout: Duration,
}

impl AgentConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let raw =
            fs::read_to_string(path).with_context(|| format!("read config {}", path.display()))?;
        let config: Self =
            toml::from_str(&raw).with_context(|| format!("parse config {}", path.display()))?;
        Ok(config)
    }

    fn registration_direct_endpoint(&self) -> Option<DirectEndpoint> {
        match (self.advertise_host.as_deref(), self.listen_port) {
            (Some(host), Some(port)) if !host.trim().is_empty() => Some(DirectEndpoint {
                host: host.trim().to_string(),
                port: u32::from(port),
            }),
            _ => None,
        }
    }

    fn linux_tunnel_settings(&self) -> Option<LinuxTunnelSettings> {
        match (self.private_key.as_deref(), self.listen_port) {
            (Some(private_key), Some(listen_port)) if !private_key.trim().is_empty() => {
                Some(LinuxTunnelSettings {
                    private_key: private_key.trim().to_string(),
                    listen_port,
                })
            }
            _ => None,
        }
    }

    pub(crate) fn resolved_stun_addr(&self) -> Option<String> {
        if let Some(stun_addr) = self.stun_addr.as_ref() {
            return Some(stun_addr.clone());
        }

        let signal_addr = self
            .signal_addr
            .as_deref()?
            .trim_start_matches("http://")
            .trim_start_matches("https://");
        let host = signal_addr
            .rsplit_once(':')
            .map(|(host, _)| host)
            .unwrap_or(signal_addr);
        Some(format!("{host}:3479"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LinuxTunnelSettings {
    private_key: String,
    listen_port: u16,
}

enum RuntimeWireGuardBackend {
    Linux(LinuxWireGuardBackend),
    Windows(WindowsWireGuardBackend),
}

impl WireGuardBackend for RuntimeWireGuardBackend {
    fn reconcile(&self, desired: &wg_manager::DesiredState) -> Result<()> {
        match self {
            Self::Linux(backend) => backend.reconcile(desired),
            Self::Windows(backend) => backend.reconcile(desired),
        }
    }
}

pub async fn run(config: AgentConfig) -> Result<()> {
    log_data_plane_mode(&config);

    loop {
        match register_and_sync(&config).await {
            Ok(()) => warn!("sync stream closed, reconnecting"),
            Err(err) => error!("agent cycle failed: {err:#}"),
        }

        sleep(Duration::from_secs(2)).await;
    }
}

async fn register_and_sync(config: &AgentConfig) -> Result<()> {
    let endpoint = api_client_endpoint(&config.management_addr);
    let mut client = ManagementServiceClient::connect(endpoint)
        .await
        .context("connect management service")?;
    let mut peer_cache = PeerCache::default();
    let backend = build_runtime_backend(config);

    let response = client
        .register_device(build_register_device_request(config))
        .await
        .context("register device")?
        .into_inner();

    let register_peers = response.peers.len();
    let device = response
        .device
        .ok_or_else(|| anyhow!("management response missing device"))?;
    let overlay = device
        .overlay
        .as_ref()
        .map(|overlay| overlay.ipv4.clone())
        .unwrap_or_default();

    info!(
        device_id = %device.id,
        interface = %config.interface_name,
        overlay_ipv4 = %overlay,
        peers = register_peers,
        advertised_routes = config.advertised_routes.len(),
        "device registered"
    );

    let mut signal_runtime = signal::SignalRuntime::spawn(config.clone(), device.id.clone());
    let mut endpoint_overrides = BTreeMap::<String, Endpoint>::new();
    let mut last_event: Option<SyncConfigEvent> = None;

    let mut stream = client
        .sync_config(SyncConfigRequest {
            device_id: device.id.clone(),
        })
        .await
        .context("open config stream")?
        .into_inner();

    loop {
        let signal_update = async {
            match signal_runtime.as_mut() {
                Some(runtime) => runtime.recv_update().await,
                None => pending().await,
            }
        };

        tokio::select! {
            event = stream.message() => {
                let Some(event) = event.context("receive config event")? else {
                    break;
                };
        let event_type = describe_event_type(event.r#type);
        let update = peer_cache.apply_event(&event);
        if !update.applied {
            info!(
                event_type,
                revision = %event.revision,
                current_revision = %update.current_revision,
                "ignored stale config event"
            );
            continue;
        }

        for change in &update.changes {
            info!(
                event_type,
                revision = %event.revision,
                peer_id = %change.peer_id,
                change = %change.kind,
                "peer cache updated"
            );
        }

        let snapshot = peer_cache.snapshot();
        if let Some(runtime) = signal_runtime.as_ref() {
            runtime.publish_snapshot(snapshot.clone());
        }
        info!(
            event_type,
            revision = %snapshot.revision,
            peers = event.peers.len(),
            tracked_peers = snapshot.peers.len(),
            peer_added = update.added,
            peer_updated = update.updated,
            peer_removed = update.removed,
            "received config event"
        );

        last_event = Some(event.clone());
        reconcile_wireguard_state(config, backend.as_ref(), &event, &endpoint_overrides)?;
            }
            update = signal_update => {
                let Some(update) = update else {
                    continue;
                };
                match update.endpoint_override {
                    Some(endpoint) => {
                        endpoint_overrides.insert(update.peer_id.clone(), endpoint);
                    }
                    None => {
                        endpoint_overrides.remove(&update.peer_id);
                    }
                }

                info!(
                    peer_id = %update.peer_id,
                    reason = %update.reason,
                    active_overrides = endpoint_overrides.len(),
                    "signal update applied"
                );

                if let Some(event) = last_event.as_ref() {
                    reconcile_wireguard_state(config, backend.as_ref(), event, &endpoint_overrides)?;
                    if let Some(probe_overlay_ipv4) = update.probe_overlay_ipv4.as_deref() {
                        trigger_wireguard_probe(probe_overlay_ipv4);
                    }
                }
            }
        }
    }

    Ok(())
}

fn reconcile_wireguard_state(
    config: &AgentConfig,
    backend: Option<&RuntimeWireGuardBackend>,
    event: &SyncConfigEvent,
    endpoint_overrides: &BTreeMap<String, Endpoint>,
) -> Result<()> {
    let Some(backend) = backend else {
        return Ok(());
    };

    let Some(settings) = config.linux_tunnel_settings() else {
        return Ok(());
    };

    let self_device = event
        .self_
        .as_ref()
        .ok_or_else(|| anyhow!("config event missing self device"))?;
    let outcome = build_desired_state_with_overrides(
        &config.interface_name,
        &settings.private_key,
        settings.listen_port,
        self_device,
        &event.peers,
        endpoint_overrides,
    )
    .context("build wireguard desired state")?;

    for skipped in &outcome.skipped_peers {
        warn!(
            peer_id = %skipped.peer_id,
            reason = %skipped.reason,
            "skipping peer during wireguard reconciliation"
        );
    }

    backend
        .reconcile(&outcome.desired_state)
        .context("reconcile wireguard state")?;

    info!(
        interface = %outcome.desired_state.interface_name,
        overlay_ipv4 = %outcome.desired_state.address_cidr,
        configured_peers = outcome.desired_state.peers.len(),
        skipped_peers = outcome.skipped_peers.len(),
        "wireguard state reconciled"
    );

    Ok(())
}

fn log_data_plane_mode(config: &AgentConfig) {
    match (
        config.private_key.is_some(),
        config.listen_port.is_some(),
        config.supports_runtime_wireguard(),
    ) {
        (true, true, true) => info!(
            interface = %config.interface_name,
            listen_port = config.listen_port.unwrap_or_default(),
            advertised_routes = config.advertised_routes.len(),
            os = %config.os,
            "wireguard reconciliation enabled"
        ),
        (false, false, false) => info!("running in discovery-only mode"),
        _ => warn!(
            interface = %config.interface_name,
            "partial data-plane config detected; running in discovery-only mode"
        ),
    }
}

fn build_register_device_request(config: &AgentConfig) -> RegisterDeviceRequest {
    RegisterDeviceRequest {
        name: config.node_name.clone(),
        public_key: config.public_key.clone(),
        token: config.bootstrap_token.clone(),
        os: config.os.clone(),
        version: config.version.clone(),
        direct_endpoint: config.registration_direct_endpoint(),
        advertised_routes: config.advertised_routes.clone(),
    }
}

fn build_runtime_backend(config: &AgentConfig) -> Option<RuntimeWireGuardBackend> {
    if !config.supports_runtime_wireguard()
        || config.private_key.is_none()
        || config.listen_port.is_none()
    {
        return None;
    }

    match config.os.as_str() {
        "linux" => Some(RuntimeWireGuardBackend::Linux(LinuxWireGuardBackend::new())),
        "windows" => Some(RuntimeWireGuardBackend::Windows(
            WindowsWireGuardBackend::new(),
        )),
        _ => None,
    }
}

fn trigger_wireguard_probe(overlay_ipv4: &str) {
    if overlay_ipv4.trim().is_empty() {
        return;
    }

    let ping_bin = if std::path::Path::new("/usr/bin/ping").exists() {
        "/usr/bin/ping"
    } else {
        "ping"
    };

    if let Err(err) = Command::new(ping_bin)
        .args(["-c", "1", "-W", "1", overlay_ipv4])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        warn!(overlay_ipv4, "failed to trigger wireguard probe: {err}");
    }
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_punch_timeout() -> Duration {
    Duration::from_secs(3)
}

fn default_os() -> String {
    std::env::consts::OS.to_string()
}

fn default_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn deserialize_duration<'de, D>(deserializer: D) -> std::result::Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    match value {
        None => Ok(default_punch_timeout()),
        Some(raw) => parse_duration(&raw).map_err(D::Error::custom),
    }
}

fn parse_duration(raw: &str) -> Result<Duration> {
    let trimmed = raw.trim();
    if let Some(value) = trimmed.strip_suffix("ms") {
        return Ok(Duration::from_millis(value.trim().parse()?));
    }
    if let Some(value) = trimmed.strip_suffix('s') {
        return Ok(Duration::from_secs(value.trim().parse()?));
    }
    if let Some(value) = trimmed.strip_suffix('m') {
        return Ok(Duration::from_secs(value.trim().parse::<u64>()? * 60));
    }
    Ok(Duration::from_secs(trimmed.parse()?))
}

pub(crate) fn api_client_endpoint(addr: &str) -> String {
    api_client::normalize_endpoint(addr)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedPeer {
    pub peer_id: String,
    pub public_key: String,
    pub overlay_ipv4: String,
    pub overlay_ipv6: String,
    pub allowed_ips: Vec<String>,
    pub preferred_path: i32,
    pub direct_endpoint: Option<CachedDirectEndpoint>,
}

impl CachedPeer {
    fn from_proto(peer: &Peer) -> Self {
        let overlay = peer.overlay.as_ref();
        Self {
            peer_id: peer.peer_id.clone(),
            public_key: peer.public_key.clone(),
            overlay_ipv4: overlay.map(|value| value.ipv4.clone()).unwrap_or_default(),
            overlay_ipv6: overlay.map(|value| value.ipv6.clone()).unwrap_or_default(),
            allowed_ips: peer.allowed_ips.clone(),
            preferred_path: peer.preferred_path,
            direct_endpoint: peer
                .direct_endpoint
                .as_ref()
                .map(CachedDirectEndpoint::from_proto),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedDirectEndpoint {
    pub host: String,
    pub port: u32,
}

impl CachedDirectEndpoint {
    fn from_proto(endpoint: &DirectEndpoint) -> Self {
        Self {
            host: endpoint.host.clone(),
            port: endpoint.port,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PeerSnapshot {
    pub revision: String,
    pub peers: Vec<CachedPeer>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PeerChange {
    kind: &'static str,
    peer_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PeerCacheUpdate {
    applied: bool,
    current_revision: String,
    tracked_peers: usize,
    added: usize,
    updated: usize,
    removed: usize,
    changes: Vec<PeerChange>,
}

#[derive(Debug, Default)]
struct PeerCache {
    revision: String,
    peers: BTreeMap<String, CachedPeer>,
}

impl PeerCache {
    fn apply_event(&mut self, event: &SyncConfigEvent) -> PeerCacheUpdate {
        if !self.revision.is_empty() && event.revision <= self.revision {
            return PeerCacheUpdate {
                applied: false,
                current_revision: self.revision.clone(),
                tracked_peers: self.peers.len(),
                added: 0,
                updated: 0,
                removed: 0,
                changes: Vec::new(),
            };
        }

        let mut next = BTreeMap::new();
        for peer in &event.peers {
            if peer.peer_id.is_empty() {
                continue;
            }
            next.insert(peer.peer_id.clone(), CachedPeer::from_proto(peer));
        }

        let mut changes = Vec::new();
        let mut added = 0;
        let mut updated = 0;
        let mut removed = 0;

        for (peer_id, next_peer) in &next {
            match self.peers.get(peer_id) {
                None => {
                    added += 1;
                    changes.push(PeerChange {
                        kind: "added",
                        peer_id: peer_id.clone(),
                    });
                }
                Some(current_peer) if current_peer != next_peer => {
                    updated += 1;
                    changes.push(PeerChange {
                        kind: "updated",
                        peer_id: peer_id.clone(),
                    });
                }
                Some(_) => {}
            }
        }

        for peer_id in self.peers.keys() {
            if !next.contains_key(peer_id) {
                removed += 1;
                changes.push(PeerChange {
                    kind: "removed",
                    peer_id: peer_id.clone(),
                });
            }
        }

        self.revision = event.revision.clone();
        self.peers = next;

        PeerCacheUpdate {
            applied: true,
            current_revision: self.revision.clone(),
            tracked_peers: self.peers.len(),
            added,
            updated,
            removed,
            changes,
        }
    }

    fn snapshot(&self) -> PeerSnapshot {
        PeerSnapshot {
            revision: self.revision.clone(),
            peers: self.peers.values().cloned().collect(),
        }
    }
}

fn describe_event_type(raw: i32) -> &'static str {
    match SyncConfigEventType::try_from(raw).ok() {
        Some(SyncConfigEventType::Full) => "full",
        Some(SyncConfigEventType::Incremental) => "incremental",
        _ => "unspecified",
    }
}

impl AgentConfig {
    fn supports_runtime_wireguard(&self) -> bool {
        matches!(self.os.as_str(), "linux" | "windows")
    }
}

#[cfg(test)]
mod tests {
    use super::{build_register_device_request, AgentConfig, CachedPeer, PeerCache};
    use api_client::proto::{OverlayAddress, PathType, Peer, SyncConfigEvent, SyncConfigEventType};
    use std::time::Duration;

    #[test]
    fn agent_config_parses_optional_linux_fields() {
        let config: AgentConfig = toml::from_str(
            r#"
node_name = "client-a"
management_addr = "127.0.0.1:33073"
signal_addr = "127.0.0.1:10000"
relay_addr = "127.0.0.1:3478"
bootstrap_token = "meshlink-dev-token"
public_key = "meshlink-client-a-public-key"
interface_name = "sdwan0"
private_key = "meshlink-client-a-private-key"
listen_port = 51820
advertise_host = "192.0.2.10"
advertised_routes = ["10.20.0.0/24", "10.30.0.0/24"]
punch_timeout = "5s"
"#,
        )
        .expect("parse config");

        let endpoint = config
            .registration_direct_endpoint()
            .expect("registration direct endpoint");
        let settings = config
            .linux_tunnel_settings()
            .expect("linux tunnel settings");

        assert_eq!(endpoint.host, "192.0.2.10");
        assert_eq!(endpoint.port, 51820);
        assert_eq!(settings.listen_port, 51820);
        assert_eq!(config.relay_addr.as_deref(), Some("127.0.0.1:3478"));
        assert_eq!(
            config.advertised_routes,
            vec!["10.20.0.0/24".to_string(), "10.30.0.0/24".to_string()]
        );
        assert_eq!(
            config.resolved_stun_addr().as_deref(),
            Some("127.0.0.1:3479")
        );
        assert_eq!(config.punch_timeout, Duration::from_secs(5));
    }

    #[test]
    fn missing_linux_fields_leave_discovery_only_mode() {
        let config: AgentConfig = toml::from_str(
            r#"
node_name = "client-a"
management_addr = "127.0.0.1:33073"
bootstrap_token = "meshlink-dev-token"
public_key = "meshlink-client-a-public-key"
interface_name = "sdwan0"
"#,
        )
        .expect("parse config");

        assert!(config.registration_direct_endpoint().is_none());
        assert!(config.linux_tunnel_settings().is_none());
    }

    #[test]
    fn register_device_request_includes_advertised_routes() {
        let config: AgentConfig = toml::from_str(
            r#"
node_name = "client-a"
management_addr = "127.0.0.1:33073"
bootstrap_token = "meshlink-dev-token"
public_key = "meshlink-client-a-public-key"
interface_name = "sdwan0"
advertised_routes = ["10.20.0.0/24"]
"#,
        )
        .expect("parse config");

        let request = build_register_device_request(&config);
        assert_eq!(request.advertised_routes, vec!["10.20.0.0/24".to_string()]);
    }

    #[test]
    fn full_event_populates_peer_cache() {
        let mut cache = PeerCache::default();

        let update = cache.apply_event(&SyncConfigEvent {
            r#type: SyncConfigEventType::Full as i32,
            revision: "00000000000000000001".to_string(),
            peers: vec![peer("dev-b", "pk-b", "100.64.0.2")],
            ..Default::default()
        });

        assert!(update.applied);
        assert_eq!(update.added, 1);
        assert_eq!(update.tracked_peers, 1);
        assert_eq!(
            cache.snapshot().peers,
            vec![CachedPeer {
                peer_id: "dev-b".to_string(),
                public_key: "pk-b".to_string(),
                overlay_ipv4: "100.64.0.2".to_string(),
                overlay_ipv6: String::new(),
                allowed_ips: vec!["100.64.0.2/32".to_string()],
                preferred_path: PathType::PublicIpv4 as i32,
                direct_endpoint: None,
            }]
        );
    }

    #[test]
    fn newer_event_replaces_existing_view() {
        let mut cache = PeerCache::default();
        cache.apply_event(&SyncConfigEvent {
            r#type: SyncConfigEventType::Full as i32,
            revision: "00000000000000000001".to_string(),
            peers: vec![peer("dev-b", "pk-b", "100.64.0.2")],
            ..Default::default()
        });

        let update = cache.apply_event(&SyncConfigEvent {
            r#type: SyncConfigEventType::Incremental as i32,
            revision: "00000000000000000002".to_string(),
            peers: vec![peer("dev-b", "pk-b-new", "100.64.0.22")],
            ..Default::default()
        });

        assert!(update.applied);
        assert_eq!(update.updated, 1);
        assert_eq!(cache.snapshot().revision, "00000000000000000002");
        assert_eq!(cache.snapshot().peers[0].public_key, "pk-b-new");
        assert_eq!(cache.snapshot().peers[0].overlay_ipv4, "100.64.0.22");
    }

    #[test]
    fn missing_peer_is_removed_from_cache() {
        let mut cache = PeerCache::default();
        cache.apply_event(&SyncConfigEvent {
            r#type: SyncConfigEventType::Full as i32,
            revision: "00000000000000000001".to_string(),
            peers: vec![
                peer("dev-b", "pk-b", "100.64.0.2"),
                peer("dev-c", "pk-c", "100.64.0.3"),
            ],
            ..Default::default()
        });

        let update = cache.apply_event(&SyncConfigEvent {
            r#type: SyncConfigEventType::Incremental as i32,
            revision: "00000000000000000002".to_string(),
            peers: vec![peer("dev-c", "pk-c", "100.64.0.3")],
            ..Default::default()
        });

        assert!(update.applied);
        assert_eq!(update.removed, 1);
        assert_eq!(cache.snapshot().peers.len(), 1);
        assert_eq!(cache.snapshot().peers[0].peer_id, "dev-c");
    }

    #[test]
    fn stale_revision_is_ignored() {
        let mut cache = PeerCache::default();
        cache.apply_event(&SyncConfigEvent {
            r#type: SyncConfigEventType::Full as i32,
            revision: "00000000000000000002".to_string(),
            peers: vec![peer("dev-b", "pk-b", "100.64.0.2")],
            ..Default::default()
        });

        let update = cache.apply_event(&SyncConfigEvent {
            r#type: SyncConfigEventType::Incremental as i32,
            revision: "00000000000000000001".to_string(),
            peers: vec![],
            ..Default::default()
        });

        assert!(!update.applied);
        assert_eq!(cache.snapshot().peers.len(), 1);
    }

    fn peer(peer_id: &str, public_key: &str, overlay_ipv4: &str) -> Peer {
        Peer {
            peer_id: peer_id.to_string(),
            public_key: public_key.to_string(),
            overlay: Some(OverlayAddress {
                ipv4: overlay_ipv4.to_string(),
                ipv6: String::new(),
            }),
            allowed_ips: vec![format!("{overlay_ipv4}/32")],
            preferred_path: PathType::PublicIpv4 as i32,
            ..Default::default()
        }
    }
}
