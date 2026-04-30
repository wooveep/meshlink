#![cfg_attr(not(windows), allow(dead_code))]

use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(windows)]
use std::{
    ffi::OsStr,
    fs,
    io::{self, BufRead, Write},
    os::windows::ffi::OsStrExt,
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};

use anyhow::{anyhow, bail, Context, Result};
use wg_manager::{DesiredState, WireGuardBackend};

#[cfg_attr(not(windows), allow(dead_code))]
const WINDOWS_RUNTIME_VERSION: &str = "v0.3.17";

#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsWireGuardBackend;

#[derive(Debug, Clone, PartialEq, Eq)]
struct PeerRuntimeState {
    endpoint: Option<wg_manager::Endpoint>,
    last_handshake_timestamp: u64,
}

const WG_KEY_LEN: usize = 32;
const WIREGUARD_SOCKADDR_INET_LEN: usize = 28;
const FILETIME_UNIX_EPOCH_OFFSET_100NS: u64 = 116_444_736_000_000_000;
const AF_INET: u16 = 2;
const AF_INET6: u16 = 23;
const WIREGUARD_INTERFACE_HAS_PRIVATE_KEY: u32 = 1 << 1;
const WIREGUARD_INTERFACE_HAS_LISTEN_PORT: u32 = 1 << 2;
const WIREGUARD_INTERFACE_REPLACE_PEERS: u32 = 1 << 3;
const WIREGUARD_PEER_HAS_PUBLIC_KEY: u32 = 1 << 0;
const WIREGUARD_PEER_HAS_PERSISTENT_KEEPALIVE: u32 = 1 << 2;
const WIREGUARD_PEER_HAS_ENDPOINT: u32 = 1 << 3;
const WIREGUARD_PEER_REPLACE_ALLOWED_IPS: u32 = 1 << 5;
const WIREGUARD_PEER_UPDATE_ONLY: u32 = 1 << 7;

#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct WireGuardInterface {
    flags: u32,
    listen_port: u16,
    private_key: [u8; WG_KEY_LEN],
    public_key: [u8; WG_KEY_LEN],
    peers_count: u32,
}

#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct WireGuardPeer {
    flags: u32,
    reserved: u32,
    public_key: [u8; WG_KEY_LEN],
    preshared_key: [u8; WG_KEY_LEN],
    persistent_keepalive: u16,
    _endpoint_padding: u16,
    endpoint: [u8; WIREGUARD_SOCKADDR_INET_LEN],
    tx_bytes: u64,
    rx_bytes: u64,
    last_handshake: u64,
    allowed_ips_count: u32,
}

#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct WireGuardAllowedIp {
    flags: u32,
    address: [u8; 16],
    address_family: u16,
    cidr: u8,
    _padding: u8,
}

pub fn latest_handshake_timestamp(interface_name: &str, peer_public_key: &str) -> Result<u64> {
    Ok(load_peer_runtime_state(interface_name, peer_public_key)?
        .map(|state| state.last_handshake_timestamp)
        .unwrap_or_default())
}

pub fn peer_endpoint(
    interface_name: &str,
    peer_public_key: &str,
) -> Result<Option<wg_manager::Endpoint>> {
    Ok(load_peer_runtime_state(interface_name, peer_public_key)?.and_then(|state| state.endpoint))
}

impl WindowsWireGuardBackend {
    pub fn new() -> Self {
        Self
    }
}

impl WireGuardBackend for WindowsWireGuardBackend {
    fn reconcile(&self, desired: &DesiredState) -> Result<()> {
        reconcile_windows(desired)
    }
}

pub fn run_embedded_tunnel_service(config_path: &Path) -> Result<()> {
    run_windows_service(config_path)
}

#[cfg_attr(not(windows), allow(dead_code))]
fn render_tunnel_config(desired: &DesiredState) -> String {
    let mut rendered = format!(
        "[Interface]\nPrivateKey = {}\nListenPort = {}\nAddress = {}\n",
        desired.private_key, desired.listen_port, desired.address_cidr
    );

    for peer in &desired.peers {
        rendered.push_str("\n[Peer]\n");
        rendered.push_str(&format!("PublicKey = {}\n", peer.public_key));
        rendered.push_str(&format!("Endpoint = {}\n", peer.endpoint.render()));
        rendered.push_str(&format!("AllowedIPs = {}\n", peer.allowed_ips.join(", ")));
        if let Some(seconds) = peer.persistent_keepalive_seconds {
            rendered.push_str(&format!("PersistentKeepalive = {seconds}\n"));
        }
    }

    rendered
}

#[cfg(windows)]
fn reconcile_windows(desired: &DesiredState) -> Result<()> {
    let config_path = stable_config_path(&desired.interface_name);
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create windows wireguard config dir {}", parent.display()))?;
    }
    let rendered = render_tunnel_config(desired);
    let existing = fs::read_to_string(&config_path).ok();
    let config_changed = !matches!(existing.as_deref(), Some(current) if current == rendered);
    let endpoint_only_change = config_changed
        && existing
            .as_deref()
            .map(|current| equivalent_except_endpoints(current, &rendered))
            .unwrap_or(false);
    ensure_runtime_assets_present()?;
    ensure_tunnel_service(
        desired,
        &config_path,
        &rendered,
        config_changed,
        endpoint_only_change,
    )
}

#[cfg(not(windows))]
fn reconcile_windows(_desired: &DesiredState) -> Result<()> {
    bail!("windows wireguard backend is only available on Windows")
}

#[cfg(windows)]
fn run_windows_service(config_path: &Path) -> Result<()> {
    let tunnel_path = resolve_runtime_asset("tunnel.dll")?;
    let config_wide = wide_null(config_path.as_os_str());

    unsafe {
        let library = libloading::Library::new(&tunnel_path)
            .with_context(|| format!("load tunnel service library {}", tunnel_path.display()))?;
        let service: libloading::Symbol<unsafe extern "system" fn(*const u16) -> u32> = library
            .get(b"WireGuardTunnelService\0")
            .context("resolve WireGuardTunnelService export")?;
        let exit_code = service(config_wide.as_ptr());
        if exit_code == 0 {
            Ok(())
        } else {
            bail!(
                "WireGuardTunnelService returned non-zero status {} for {}",
                exit_code,
                config_path.display()
            )
        }
    }
}

#[cfg(not(windows))]
fn run_windows_service(_config_path: &Path) -> Result<()> {
    bail!("embedded tunnel service is only available on Windows")
}

#[cfg(windows)]
fn load_peer_runtime_state(
    interface_name: &str,
    peer_public_key: &str,
) -> Result<Option<PeerRuntimeState>> {
    let target_public_key = decode_public_key(peer_public_key)?;
    match load_peer_runtime_state_from_uapi(interface_name, &target_public_key) {
        Ok(state) => return Ok(state),
        Err(err) => {
            eprintln!(
                "tunnel UAPI runtime state read failed for {}; falling back to WireGuardNT adapter API: {err:#}",
                interface_name
            );
        }
    }

    let api = WireGuardApi::load()?;
    let adapter = match api.open_adapter_with_retry(interface_name)? {
        Some(adapter) => adapter,
        None => return Ok(None),
    };
    let config = api.read_configuration(&adapter)?;
    parse_peer_runtime_state(&config, &target_public_key)
}

#[cfg(not(windows))]
fn load_peer_runtime_state(
    _interface_name: &str,
    _peer_public_key: &str,
) -> Result<Option<PeerRuntimeState>> {
    Ok(None)
}

#[cfg(windows)]
fn ensure_tunnel_service(
    desired: &DesiredState,
    config_path: &Path,
    rendered_config: &str,
    config_changed: bool,
    endpoint_only_change: bool,
) -> Result<()> {
    let service_name = service_name(&desired.interface_name);
    let display_name = display_name(&desired.interface_name);
    let binary = std::env::current_exe().context("resolve meshlinkd.exe path")?;
    let bin_path = service_bin_path(&binary, config_path);
    let sc = resolve_sc_exe();

    if !service_exists(&service_name)? {
        write_tunnel_config(config_path, rendered_config)?;
        run_checked(
            &sc,
            &[
                "create",
                service_name.as_str(),
                "type=",
                "own",
                "start=",
                "auto",
                "error=",
                "normal",
                "binPath=",
                bin_path.as_str(),
                "DisplayName=",
                display_name.as_str(),
                "depend=",
                "Nsi/TcpIp",
            ],
        )?;
        run_checked(&sc, &["sidtype", service_name.as_str(), "unrestricted"])?;
    }

    if config_changed && service_running(&service_name)? {
        if endpoint_only_change {
            match apply_endpoint_runtime_config(desired) {
                Ok(()) => {
                    write_tunnel_config(config_path, rendered_config)?;
                    return Ok(());
                }
                Err(err) => {
                    eprintln!(
                        "runtime endpoint-only update failed for {}; keeping tunnel service running and writing config for next service start: {err:#}",
                        desired.interface_name
                    );
                    write_tunnel_config(config_path, rendered_config)?;
                    return Ok(());
                }
            }
        }
        write_tunnel_config(config_path, rendered_config)?;
        run_checked(&sc, &["stop", service_name.as_str()])?;
        wait_for_service_state(&service_name, "STOPPED")?;
    } else if config_changed {
        write_tunnel_config(config_path, rendered_config)?;
    }

    if !service_running(&service_name)? {
        run_checked(&sc, &["start", service_name.as_str()])?;
        wait_for_service_state(&service_name, "RUNNING")?;
    }
    Ok(())
}

#[cfg(windows)]
fn service_exists(service_name: &str) -> Result<bool> {
    let output = run(&resolve_sc_exe(), &["query", service_name])?;
    Ok(output.status.success())
}

#[cfg(windows)]
fn service_running(service_name: &str) -> Result<bool> {
    let output = run_checked_stdout(&resolve_sc_exe(), &["query", service_name])?;
    Ok(parse_service_state(&output)
        .map(|state| state.eq_ignore_ascii_case("RUNNING"))
        .unwrap_or(false))
}

#[cfg(windows)]
fn wait_for_service_state(service_name: &str, expected: &str) -> Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        let output = run_checked_stdout(&resolve_sc_exe(), &["query", service_name])?;
        if parse_service_state(&output)
            .map(|state| state.eq_ignore_ascii_case(expected))
            .unwrap_or(false)
        {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }

    bail!(
        "timed out waiting for service {} to reach state {}",
        service_name,
        expected
    )
}

#[cfg_attr(not(windows), allow(dead_code))]
fn stable_config_path(interface_name: &str) -> PathBuf {
    #[cfg(windows)]
    {
        let base = std::env::var_os("ProgramData")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        return base.join("MeshLink").join(format!("{interface_name}.conf"));
    }

    #[cfg(not(windows))]
    {
        std::env::temp_dir().join(format!("{interface_name}.conf"))
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
fn service_name(interface_name: &str) -> String {
    format!("WireGuardTunnel${interface_name}")
}

#[cfg_attr(not(windows), allow(dead_code))]
fn display_name(interface_name: &str) -> String {
    format!("MeshLink Tunnel ({interface_name})")
}

#[cfg(windows)]
fn write_tunnel_config(path: &Path, rendered: &str) -> Result<()> {
    fs::write(path, rendered)
        .with_context(|| format!("write windows wireguard config {}", path.display()))
}

#[cfg(windows)]
fn service_bin_path(binary: &Path, config_path: &Path) -> String {
    format!(
        "\"{}\" /service \"{}\"",
        binary.display(),
        config_path.display()
    )
}

#[cfg(windows)]
fn ensure_runtime_assets_present() -> Result<()> {
    resolve_runtime_asset("tunnel.dll")?;
    resolve_runtime_asset("wireguard.dll")?;
    resolve_runtime_asset("wintun.dll")?;
    Ok(())
}

#[cfg(windows)]
fn resolve_runtime_asset(filename: &str) -> Result<PathBuf> {
    let exe_dir = std::env::current_exe()
        .context("resolve meshlinkd.exe path")?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("resolve meshlinkd.exe directory"))?;

    let local = exe_dir.join(filename);
    if local.exists() {
        return Ok(local);
    }

    let repo_pinned = exe_dir
        .join("runtime")
        .join(WINDOWS_RUNTIME_VERSION)
        .join(filename);
    if repo_pinned.exists() {
        return Ok(repo_pinned);
    }

    bail!(
        "required runtime asset '{}' was not found next to meshlinkd.exe or under runtime/{}",
        filename,
        WINDOWS_RUNTIME_VERSION
    )
}

#[cfg(windows)]
fn resolve_sc_exe() -> String {
    let candidate = PathBuf::from(r"C:\Windows\System32\sc.exe");
    if candidate.exists() {
        candidate.display().to_string()
    } else {
        "sc.exe".to_string()
    }
}

#[cfg(windows)]
fn wide_null(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
type WireGuardAdapterHandle = *mut std::ffi::c_void;

#[cfg(windows)]
type WireGuardOpenAdapterFn = unsafe extern "system" fn(*const u16) -> WireGuardAdapterHandle;

#[cfg(windows)]
type WireGuardCloseAdapterFn = unsafe extern "system" fn(WireGuardAdapterHandle);

#[cfg(windows)]
type WireGuardGetConfigurationFn =
    unsafe extern "system" fn(WireGuardAdapterHandle, *mut WireGuardInterface, *mut u32) -> i32;

#[cfg(windows)]
type WireGuardSetConfigurationFn =
    unsafe extern "system" fn(WireGuardAdapterHandle, *const WireGuardInterface, u32) -> i32;

#[cfg(windows)]
struct WireGuardApi {
    _library: libloading::Library,
    open_adapter: WireGuardOpenAdapterFn,
    close_adapter: WireGuardCloseAdapterFn,
    get_configuration: WireGuardGetConfigurationFn,
    set_configuration: WireGuardSetConfigurationFn,
}

#[cfg(windows)]
struct AdapterGuard {
    handle: WireGuardAdapterHandle,
    close_adapter: WireGuardCloseAdapterFn,
}

#[cfg(windows)]
impl Drop for AdapterGuard {
    fn drop(&mut self) {
        unsafe {
            (self.close_adapter)(self.handle);
        }
    }
}

#[cfg(windows)]
impl WireGuardApi {
    fn load() -> Result<Self> {
        unsafe {
            let library = libloading::Library::new(resolve_runtime_asset("wireguard.dll")?)
                .context("load wireguard runtime library")?;
            let open_adapter = *library
                .get::<WireGuardOpenAdapterFn>(b"WireGuardOpenAdapter\0")
                .context("resolve WireGuardOpenAdapter export")?;
            let close_adapter = *library
                .get::<WireGuardCloseAdapterFn>(b"WireGuardCloseAdapter\0")
                .context("resolve WireGuardCloseAdapter export")?;
            let get_configuration = *library
                .get::<WireGuardGetConfigurationFn>(b"WireGuardGetConfiguration\0")
                .context("resolve WireGuardGetConfiguration export")?;
            let set_configuration = *library
                .get::<WireGuardSetConfigurationFn>(b"WireGuardSetConfiguration\0")
                .context("resolve WireGuardSetConfiguration export")?;
            Ok(Self {
                _library: library,
                open_adapter,
                close_adapter,
                get_configuration,
                set_configuration,
            })
        }
    }

    fn open_adapter(&self, interface_name: &str) -> Result<Option<AdapterGuard>> {
        let interface = wide_null(OsStr::new(interface_name));
        let handle = unsafe { (self.open_adapter)(interface.as_ptr()) };
        if handle.is_null() {
            let err = io::Error::last_os_error();
            return match err.raw_os_error() {
                Some(2) | Some(1168) => Ok(None),
                _ => Err(err).with_context(|| format!("open WireGuard adapter {}", interface_name)),
            };
        }

        Ok(Some(AdapterGuard {
            handle,
            close_adapter: self.close_adapter,
        }))
    }

    fn open_adapter_with_retry(&self, interface_name: &str) -> Result<Option<AdapterGuard>> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            let adapter = self.open_adapter(interface_name)?;
            if adapter.is_some() || std::time::Instant::now() >= deadline {
                return Ok(adapter);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    fn read_configuration(&self, adapter: &AdapterGuard) -> Result<Vec<u8>> {
        let mut bytes = std::mem::size_of::<WireGuardInterface>() as u32;
        loop {
            let mut buffer = vec![0u8; bytes as usize];
            let success = unsafe {
                (self.get_configuration)(
                    adapter.handle,
                    buffer.as_mut_ptr() as *mut WireGuardInterface,
                    &mut bytes,
                )
            };
            if success != 0 {
                buffer.truncate(bytes as usize);
                return Ok(buffer);
            }

            let err = io::Error::last_os_error();
            if err.raw_os_error() == Some(234) {
                continue;
            }
            return Err(err).context("read WireGuard adapter configuration");
        }
    }

    fn set_configuration(&self, adapter: &AdapterGuard, config: &[u8]) -> Result<()> {
        let success = unsafe {
            (self.set_configuration)(
                adapter.handle,
                config.as_ptr() as *const WireGuardInterface,
                u32::try_from(config.len()).context("wireguard runtime config too large")?,
            )
        };
        if success != 0 {
            return Ok(());
        }

        Err(io::Error::last_os_error()).context("set WireGuard adapter configuration")
    }
}

#[cfg(windows)]
fn apply_runtime_config(desired: &DesiredState) -> Result<()> {
    let api = WireGuardApi::load()?;
    let adapter = api
        .open_adapter_with_retry(&desired.interface_name)?
        .ok_or_else(|| anyhow!("WireGuard adapter {} is not open", desired.interface_name))?;
    let config = build_runtime_config(desired)?;
    api.set_configuration(&adapter, &config)
}

#[cfg(not(windows))]
fn apply_runtime_config(_desired: &DesiredState) -> Result<()> {
    bail!("windows wireguard backend is only available on Windows")
}

#[cfg(windows)]
fn apply_endpoint_runtime_config(desired: &DesiredState) -> Result<()> {
    apply_endpoint_uapi_config(desired)
}

#[cfg(not(windows))]
fn apply_endpoint_runtime_config(_desired: &DesiredState) -> Result<()> {
    bail!("windows wireguard backend is only available on Windows")
}

fn equivalent_except_endpoints(left: &str, right: &str) -> bool {
    without_endpoint_lines(left) == without_endpoint_lines(right)
}

fn without_endpoint_lines(config: &str) -> String {
    config
        .lines()
        .filter(|line| !line.trim_start().starts_with("Endpoint = "))
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_runtime_config(desired: &DesiredState) -> Result<Vec<u8>> {
    let private_key = decode_key(&desired.private_key, "wireguard private key")?;
    let interface = WireGuardInterface {
        flags: WIREGUARD_INTERFACE_HAS_PRIVATE_KEY
            | WIREGUARD_INTERFACE_HAS_LISTEN_PORT
            | WIREGUARD_INTERFACE_REPLACE_PEERS,
        listen_port: desired.listen_port,
        private_key,
        public_key: [0u8; WG_KEY_LEN],
        peers_count: u32::try_from(desired.peers.len()).context("too many wireguard peers")?,
    };

    let mut config = Vec::new();
    config.extend_from_slice(as_bytes(&interface));

    for desired_peer in &desired.peers {
        let public_key = decode_key(&desired_peer.public_key, "wireguard public key")?;
        let mut peer = WireGuardPeer {
            flags: WIREGUARD_PEER_HAS_PUBLIC_KEY
                | WIREGUARD_PEER_HAS_ENDPOINT
                | WIREGUARD_PEER_REPLACE_ALLOWED_IPS,
            reserved: 0,
            public_key,
            preshared_key: [0u8; WG_KEY_LEN],
            persistent_keepalive: desired_peer
                .persistent_keepalive_seconds
                .unwrap_or_default(),
            _endpoint_padding: 0,
            endpoint: encode_endpoint(&desired_peer.endpoint)?,
            tx_bytes: 0,
            rx_bytes: 0,
            last_handshake: 0,
            allowed_ips_count: u32::try_from(desired_peer.allowed_ips.len())
                .context("too many wireguard allowed IPs")?,
        };
        if desired_peer.persistent_keepalive_seconds.is_some() {
            peer.flags |= WIREGUARD_PEER_HAS_PERSISTENT_KEEPALIVE;
        }
        config.extend_from_slice(as_bytes(&peer));

        for allowed_ip in &desired_peer.allowed_ips {
            let encoded = encode_allowed_ip(allowed_ip)?;
            config.extend_from_slice(as_bytes(&encoded));
        }
    }

    Ok(config)
}

fn build_endpoint_runtime_config(desired: &DesiredState) -> Result<Vec<u8>> {
    let interface = WireGuardInterface {
        flags: 0,
        listen_port: 0,
        private_key: [0u8; WG_KEY_LEN],
        public_key: [0u8; WG_KEY_LEN],
        peers_count: u32::try_from(desired.peers.len()).context("too many wireguard peers")?,
    };

    let mut config = Vec::new();
    config.extend_from_slice(as_bytes(&interface));

    for desired_peer in &desired.peers {
        let public_key = decode_key(&desired_peer.public_key, "wireguard public key")?;
        let peer = WireGuardPeer {
            flags: WIREGUARD_PEER_HAS_PUBLIC_KEY
                | WIREGUARD_PEER_HAS_ENDPOINT
                | WIREGUARD_PEER_UPDATE_ONLY,
            reserved: 0,
            public_key,
            preshared_key: [0u8; WG_KEY_LEN],
            persistent_keepalive: 0,
            _endpoint_padding: 0,
            endpoint: encode_endpoint(&desired_peer.endpoint)?,
            tx_bytes: 0,
            rx_bytes: 0,
            last_handshake: 0,
            allowed_ips_count: 0,
        };
        config.extend_from_slice(as_bytes(&peer));
    }

    Ok(config)
}

fn encode_endpoint(endpoint: &wg_manager::Endpoint) -> Result<[u8; WIREGUARD_SOCKADDR_INET_LEN]> {
    let host = endpoint
        .host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']');
    let ip = host
        .parse::<std::net::IpAddr>()
        .with_context(|| format!("parse wireguard endpoint host {}", endpoint.host))?;
    let mut raw = [0u8; WIREGUARD_SOCKADDR_INET_LEN];
    match ip {
        std::net::IpAddr::V4(ipv4) => {
            raw[0..2].copy_from_slice(&AF_INET.to_ne_bytes());
            raw[2..4].copy_from_slice(&endpoint.port.to_be_bytes());
            raw[4..8].copy_from_slice(&ipv4.octets());
        }
        std::net::IpAddr::V6(ipv6) => {
            raw[0..2].copy_from_slice(&AF_INET6.to_ne_bytes());
            raw[2..4].copy_from_slice(&endpoint.port.to_be_bytes());
            raw[8..24].copy_from_slice(&ipv6.octets());
        }
    }
    Ok(raw)
}

fn encode_allowed_ip(value: &str) -> Result<WireGuardAllowedIp> {
    let (address, cidr) = value
        .split_once('/')
        .ok_or_else(|| anyhow!("wireguard allowed IP missing CIDR: {value}"))?;
    let cidr = cidr
        .parse::<u8>()
        .with_context(|| format!("parse wireguard allowed IP CIDR {value}"))?;
    let ip = address
        .parse::<std::net::IpAddr>()
        .with_context(|| format!("parse wireguard allowed IP address {value}"))?;

    let mut encoded = WireGuardAllowedIp {
        flags: 0,
        address: [0u8; 16],
        address_family: 0,
        cidr,
        _padding: 0,
    };
    match ip {
        std::net::IpAddr::V4(ipv4) => {
            encoded.address_family = AF_INET;
            encoded.address[0..4].copy_from_slice(&ipv4.octets());
        }
        std::net::IpAddr::V6(ipv6) => {
            encoded.address_family = AF_INET6;
            encoded.address.copy_from_slice(&ipv6.octets());
        }
    }
    Ok(encoded)
}

fn decode_public_key(public_key: &str) -> Result<[u8; WG_KEY_LEN]> {
    decode_key(public_key, "wireguard public key")
}

fn decode_key(value: &str, label: &str) -> Result<[u8; WG_KEY_LEN]> {
    let decoded = BASE64
        .decode(value.trim())
        .with_context(|| format!("decode base64 {label}"))?;
    let key: [u8; WG_KEY_LEN] = decoded
        .try_into()
        .map_err(|_| anyhow!("{label} must be {} bytes", WG_KEY_LEN))?;
    Ok(key)
}

fn parse_peer_runtime_state(
    config: &[u8],
    target_public_key: &[u8; WG_KEY_LEN],
) -> Result<Option<PeerRuntimeState>> {
    let mut cursor = 0usize;
    let interface = read_struct::<WireGuardInterface>(config, &mut cursor)?;

    for _ in 0..interface.peers_count {
        let peer = read_struct::<WireGuardPeer>(config, &mut cursor)?;
        let endpoint = endpoint_from_raw(peer.endpoint)?;
        for _ in 0..peer.allowed_ips_count {
            let _ = read_struct::<WireGuardAllowedIp>(config, &mut cursor)?;
        }
        if &peer.public_key == target_public_key {
            return Ok(Some(PeerRuntimeState {
                endpoint,
                last_handshake_timestamp: filetime_to_unix_seconds(peer.last_handshake),
            }));
        }
    }

    Ok(None)
}

fn read_struct<T: Copy>(buffer: &[u8], cursor: &mut usize) -> Result<T> {
    let len = std::mem::size_of::<T>();
    if buffer.len().saturating_sub(*cursor) < len {
        bail!(
            "wireguard runtime config truncated: need {} bytes at offset {}, total {}",
            len,
            *cursor,
            buffer.len()
        );
    }

    let value = unsafe { std::ptr::read_unaligned(buffer[*cursor..].as_ptr() as *const T) };
    *cursor += len;
    Ok(value)
}

fn endpoint_from_raw(
    raw: [u8; WIREGUARD_SOCKADDR_INET_LEN],
) -> Result<Option<wg_manager::Endpoint>> {
    let family = u16::from_ne_bytes([raw[0], raw[1]]);
    let port = u16::from_be_bytes([raw[2], raw[3]]);
    if port == 0 {
        return Ok(None);
    }

    match family {
        AF_INET => Ok(Some(wg_manager::Endpoint {
            host: std::net::Ipv4Addr::from([raw[4], raw[5], raw[6], raw[7]]).to_string(),
            port,
        })),
        AF_INET6 => Ok(Some(wg_manager::Endpoint {
            host: std::net::Ipv6Addr::from(
                <[u8; 16]>::try_from(&raw[8..24]).context("parse IPv6 endpoint address bytes")?,
            )
            .to_string(),
            port,
        })),
        _ => Ok(None),
    }
}

fn filetime_to_unix_seconds(value: u64) -> u64 {
    if value <= FILETIME_UNIX_EPOCH_OFFSET_100NS {
        0
    } else {
        (value - FILETIME_UNIX_EPOCH_OFFSET_100NS) / 10_000_000
    }
}

fn as_bytes<T: Copy>(value: &T) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts((value as *const T).cast::<u8>(), std::mem::size_of::<T>())
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
fn run_checked(program: &str, args: &[&str]) -> Result<()> {
    let output = run(program, args)?;
    if output.status.success() {
        return Ok(());
    }

    bail!(
        "command failed: {} {}: {}",
        program,
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

#[cfg(windows)]
fn load_peer_runtime_state_from_uapi(
    interface_name: &str,
    target_public_key: &[u8; WG_KEY_LEN],
) -> Result<Option<PeerRuntimeState>> {
    let response = tunnel_uapi_request(interface_name, "get=1\n\n")?;
    ensure_uapi_success(&response)?;
    parse_peer_uapi_runtime_state(&response, target_public_key)
}

#[cfg(windows)]
fn apply_endpoint_uapi_config(desired: &DesiredState) -> Result<()> {
    let command = build_endpoint_uapi_config(desired)?;
    let response = tunnel_uapi_request(&desired.interface_name, &command)?;
    ensure_uapi_success(&response)
}

fn build_endpoint_uapi_config(desired: &DesiredState) -> Result<String> {
    let mut command = String::from("set=1\n");
    for desired_peer in &desired.peers {
        let public_key = decode_key(&desired_peer.public_key, "wireguard public key")?;
        command.push_str(&format!("public_key={}\n", hex_key(&public_key)));
        command.push_str("update_only=true\n");
        command.push_str(&format!("endpoint={}\n", desired_peer.endpoint.render()));
    }
    command.push('\n');
    Ok(command)
}

#[cfg(windows)]
fn tunnel_uapi_request(interface_name: &str, request: &str) -> Result<String> {
    let pipe_path = tunnel_uapi_pipe_path(interface_name);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let pipe = loop {
        match fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&pipe_path)
        {
            Ok(pipe) => break pipe,
            Err(err) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if err.kind() == io::ErrorKind::PermissionDenied {
                    continue;
                }
            }
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("open WireGuard tunnel UAPI pipe {pipe_path}"))
            }
        }
    };

    let mut pipe = pipe;
    pipe.write_all(request.as_bytes())
        .with_context(|| format!("write WireGuard tunnel UAPI request {pipe_path}"))?;
    pipe.flush()
        .with_context(|| format!("flush WireGuard tunnel UAPI request {pipe_path}"))?;

    let mut reader = io::BufReader::new(pipe);
    let mut response = String::new();
    loop {
        let mut line = String::new();
        let bytes = reader
            .read_line(&mut line)
            .with_context(|| format!("read WireGuard tunnel UAPI response {pipe_path}"))?;
        if bytes == 0 {
            break;
        }
        let end_of_response = line.trim_end().is_empty();
        response.push_str(&line);
        if end_of_response {
            break;
        }
    }
    Ok(response)
}

#[cfg(windows)]
fn tunnel_uapi_pipe_path(interface_name: &str) -> String {
    format!(r"\\.\pipe\ProtectedPrefix\Administrators\WireGuard\{interface_name}")
}

fn ensure_uapi_success(response: &str) -> Result<()> {
    for line in response.lines() {
        if let Some(errno) = line.strip_prefix("errno=") {
            if errno == "0" {
                return Ok(());
            }
            bail!("WireGuard tunnel UAPI returned errno={errno}: {response}");
        }
    }
    bail!("WireGuard tunnel UAPI response missing errno: {response}")
}

fn parse_peer_uapi_runtime_state(
    response: &str,
    target_public_key: &[u8; WG_KEY_LEN],
) -> Result<Option<PeerRuntimeState>> {
    let target_hex = hex_key(target_public_key);
    let mut in_target_peer = false;
    let mut found_target_peer = false;
    let mut endpoint = None;
    let mut last_handshake_timestamp = 0u64;

    for line in response.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("errno=") {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        if key == "public_key" {
            if found_target_peer && !in_target_peer {
                continue;
            }
            if found_target_peer && in_target_peer {
                break;
            }
            in_target_peer = value.eq_ignore_ascii_case(&target_hex);
            found_target_peer = in_target_peer;
            continue;
        }
        if !in_target_peer {
            continue;
        }
        match key {
            "endpoint" => endpoint = Some(parse_endpoint_value(value)?),
            "last_handshake_time_sec" => {
                last_handshake_timestamp = value
                    .parse::<u64>()
                    .with_context(|| format!("parse UAPI last_handshake_time_sec {value}"))?;
            }
            _ => {}
        }
    }

    if found_target_peer {
        Ok(Some(PeerRuntimeState {
            endpoint,
            last_handshake_timestamp,
        }))
    } else {
        Ok(None)
    }
}

fn parse_endpoint_value(value: &str) -> Result<wg_manager::Endpoint> {
    if let Some(rest) = value.strip_prefix('[') {
        let (host, port) = rest
            .split_once("]:")
            .ok_or_else(|| anyhow!("parse IPv6 WireGuard endpoint {value}"))?;
        return Ok(wg_manager::Endpoint {
            host: host.to_string(),
            port: port
                .parse::<u16>()
                .with_context(|| format!("parse WireGuard endpoint port {value}"))?,
        });
    }

    let (host, port) = value
        .rsplit_once(':')
        .ok_or_else(|| anyhow!("parse WireGuard endpoint {value}"))?;
    Ok(wg_manager::Endpoint {
        host: host.to_string(),
        port: port
            .parse::<u16>()
            .with_context(|| format!("parse WireGuard endpoint port {value}"))?,
    })
}

fn hex_key(key: &[u8; WG_KEY_LEN]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut rendered = String::with_capacity(WG_KEY_LEN * 2);
    for byte in key {
        rendered.push(HEX[(byte >> 4) as usize] as char);
        rendered.push(HEX[(byte & 0x0f) as usize] as char);
    }
    rendered
}

#[cfg_attr(not(windows), allow(dead_code))]
fn run_checked_stdout(program: &str, args: &[&str]) -> Result<String> {
    let output = run(program, args)?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }

    bail!(
        "command failed: {} {}: {}",
        program,
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

#[cfg_attr(not(windows), allow(dead_code))]
fn parse_service_state(output: &str) -> Option<&str> {
    output.lines().find_map(|line| {
        let trimmed = line.trim();
        if !trimmed.starts_with("STATE") {
            return None;
        }
        trimmed.split_whitespace().nth(3)
    })
}

#[cfg_attr(not(windows), allow(dead_code))]
fn run(program: &str, args: &[&str]) -> Result<std::process::Output> {
    Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("run {} {}", program, args.join(" ")))
}

#[cfg(test)]
mod tests {
    use super::{
        build_endpoint_runtime_config, build_runtime_config, display_name, encode_allowed_ip,
        endpoint_from_raw, equivalent_except_endpoints, filetime_to_unix_seconds, hex_key,
        parse_endpoint_value, parse_peer_runtime_state, parse_peer_uapi_runtime_state,
        parse_service_state, render_tunnel_config, service_name, stable_config_path,
        WireGuardAllowedIp, WireGuardInterface, WireGuardPeer, AF_INET, AF_INET6,
        WIREGUARD_INTERFACE_HAS_LISTEN_PORT, WIREGUARD_INTERFACE_HAS_PRIVATE_KEY,
        WIREGUARD_INTERFACE_REPLACE_PEERS, WIREGUARD_PEER_HAS_ENDPOINT,
        WIREGUARD_PEER_HAS_PERSISTENT_KEEPALIVE, WIREGUARD_PEER_HAS_PUBLIC_KEY,
        WIREGUARD_PEER_REPLACE_ALLOWED_IPS, WIREGUARD_PEER_UPDATE_ONLY,
    };
    use wg_manager::{DesiredPeer, DesiredState, Endpoint};

    #[test]
    fn render_tunnel_config_includes_interface_address_and_routes() {
        let rendered = render_tunnel_config(&DesiredState {
            interface_name: "meshlink0".to_string(),
            private_key: "private-key".to_string(),
            listen_port: 51820,
            address_cidr: "100.64.0.1/32".to_string(),
            peers: vec![DesiredPeer {
                peer_id: "dev-b".to_string(),
                public_key: "pk-b".to_string(),
                endpoint: Endpoint {
                    host: "198.51.100.20".to_string(),
                    port: 51821,
                },
                allowed_ips: vec!["100.64.0.2/32".to_string(), "10.20.0.0/24".to_string()],
                persistent_keepalive_seconds: Some(15),
            }],
        });

        assert!(rendered.contains("Address = 100.64.0.1/32"));
        assert!(rendered.contains("Endpoint = 198.51.100.20:51821"));
        assert!(rendered.contains("AllowedIPs = 100.64.0.2/32, 10.20.0.0/24"));
    }

    #[test]
    fn stable_config_path_uses_interface_name() {
        let path = stable_config_path("MeshLink");
        let rendered = path.display().to_string();
        assert!(rendered.contains("MeshLink.conf"));
    }

    #[test]
    fn service_metadata_matches_embeddable_convention() {
        assert_eq!(service_name("MeshLink"), "WireGuardTunnel$MeshLink");
        assert_eq!(display_name("MeshLink"), "MeshLink Tunnel (MeshLink)");
    }

    #[test]
    fn endpoint_only_config_comparison_ignores_endpoint_lines() {
        let left = "[Interface]\nAddress = 100.64.0.3/32\n\n[Peer]\nPublicKey = key\nEndpoint = 10.10.1.10:51820\nAllowedIPs = 100.64.0.1/32\n";
        let right = "[Interface]\nAddress = 100.64.0.3/32\n\n[Peer]\nPublicKey = key\nEndpoint = 192.168.123.211:40000\nAllowedIPs = 100.64.0.1/32\n";
        let changed_routes = "[Interface]\nAddress = 100.64.0.3/32\n\n[Peer]\nPublicKey = key\nEndpoint = 192.168.123.211:40000\nAllowedIPs = 100.64.0.1/32, 10.20.0.0/24\n";

        assert!(equivalent_except_endpoints(left, right));
        assert!(!equivalent_except_endpoints(left, changed_routes));
    }

    #[test]
    fn wireguard_nt_struct_layout_matches_c_header() {
        assert_eq!(std::mem::size_of::<WireGuardInterface>(), 80);
        assert_eq!(std::mem::size_of::<WireGuardPeer>(), 136);
        assert_eq!(std::mem::offset_of!(WireGuardPeer, endpoint), 76);
        assert_eq!(std::mem::offset_of!(WireGuardPeer, tx_bytes), 104);
        assert_eq!(std::mem::offset_of!(WireGuardPeer, allowed_ips_count), 128);
        assert_eq!(std::mem::size_of::<WireGuardAllowedIp>(), 24);
        assert_eq!(std::mem::offset_of!(WireGuardAllowedIp, flags), 0);
        assert_eq!(std::mem::offset_of!(WireGuardAllowedIp, address), 4);
        assert_eq!(std::mem::offset_of!(WireGuardAllowedIp, address_family), 20);
    }

    #[test]
    fn endpoint_from_raw_extracts_ipv4_endpoint() {
        let mut raw = [0u8; super::WIREGUARD_SOCKADDR_INET_LEN];
        raw[0..2].copy_from_slice(&AF_INET.to_ne_bytes());
        raw[2..4].copy_from_slice(&51821u16.to_be_bytes());
        raw[4..8].copy_from_slice(&[192, 0, 2, 10]);

        let endpoint = endpoint_from_raw(raw).expect("parse endpoint");
        assert_eq!(
            endpoint,
            Some(Endpoint {
                host: "192.0.2.10".to_string(),
                port: 51821,
            })
        );
    }

    #[test]
    fn encode_allowed_ip_supports_ipv4_and_ipv6_cidrs() {
        let ipv4 = encode_allowed_ip("10.20.0.0/24").expect("encode ipv4");
        assert_eq!(ipv4.address_family, AF_INET);
        assert_eq!(&ipv4.address[0..4], &[10, 20, 0, 0]);
        assert_eq!(ipv4.cidr, 24);

        let ipv6 = encode_allowed_ip("fd00::/64").expect("encode ipv6");
        assert_eq!(ipv6.address_family, AF_INET6);
        assert_eq!(ipv6.cidr, 64);
    }

    #[test]
    fn build_runtime_config_sets_wireguardnt_update_flags() {
        let desired = DesiredState {
            interface_name: "MeshLink".to_string(),
            private_key: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            listen_port: 51830,
            address_cidr: "100.64.0.3/32".to_string(),
            peers: vec![DesiredPeer {
                peer_id: "dev-a".to_string(),
                public_key: "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=".to_string(),
                endpoint: Endpoint {
                    host: "10.10.1.10".to_string(),
                    port: 51820,
                },
                allowed_ips: vec!["100.64.0.1/32".to_string(), "10.20.0.0/24".to_string()],
                persistent_keepalive_seconds: Some(15),
            }],
        };

        let config = build_runtime_config(&desired).expect("build runtime config");
        let mut cursor = 0usize;
        let interface =
            super::read_struct::<WireGuardInterface>(&config, &mut cursor).expect("read interface");
        let peer = super::read_struct::<WireGuardPeer>(&config, &mut cursor).expect("read peer");

        assert_eq!(
            interface.flags,
            WIREGUARD_INTERFACE_HAS_PRIVATE_KEY
                | WIREGUARD_INTERFACE_HAS_LISTEN_PORT
                | WIREGUARD_INTERFACE_REPLACE_PEERS
        );
        assert_eq!(interface.listen_port, 51830);
        assert_eq!(interface.peers_count, 1);
        assert_eq!(
            peer.flags,
            WIREGUARD_PEER_HAS_PUBLIC_KEY
                | WIREGUARD_PEER_HAS_ENDPOINT
                | WIREGUARD_PEER_REPLACE_ALLOWED_IPS
                | WIREGUARD_PEER_HAS_PERSISTENT_KEEPALIVE
        );
        assert_eq!(peer.allowed_ips_count, 2);
    }

    #[test]
    fn build_endpoint_runtime_config_updates_existing_peer_without_replacing_routes() {
        let desired = DesiredState {
            interface_name: "MeshLink".to_string(),
            private_key: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            listen_port: 51830,
            address_cidr: "100.64.0.3/32".to_string(),
            peers: vec![DesiredPeer {
                peer_id: "dev-a".to_string(),
                public_key: "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=".to_string(),
                endpoint: Endpoint {
                    host: "10.10.1.10".to_string(),
                    port: 51820,
                },
                allowed_ips: vec!["100.64.0.1/32".to_string(), "10.20.0.0/24".to_string()],
                persistent_keepalive_seconds: Some(15),
            }],
        };

        let config = build_endpoint_runtime_config(&desired).expect("build endpoint update");
        let mut cursor = 0usize;
        let interface =
            super::read_struct::<WireGuardInterface>(&config, &mut cursor).expect("read interface");
        let peer = super::read_struct::<WireGuardPeer>(&config, &mut cursor).expect("read peer");

        assert_eq!(interface.flags, 0);
        assert_eq!(interface.peers_count, 1);
        assert_eq!(
            peer.flags,
            WIREGUARD_PEER_HAS_PUBLIC_KEY
                | WIREGUARD_PEER_HAS_ENDPOINT
                | WIREGUARD_PEER_UPDATE_ONLY
        );
        assert_eq!(peer.allowed_ips_count, 0);
    }

    #[test]
    fn build_endpoint_uapi_config_updates_existing_peer_without_replacing_routes() {
        let desired = DesiredState {
            interface_name: "MeshLink".to_string(),
            private_key: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            listen_port: 51830,
            address_cidr: "100.64.0.3/32".to_string(),
            peers: vec![DesiredPeer {
                peer_id: "dev-a".to_string(),
                public_key: "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=".to_string(),
                endpoint: Endpoint {
                    host: "10.10.1.10".to_string(),
                    port: 51820,
                },
                allowed_ips: vec!["100.64.0.1/32".to_string(), "10.20.0.0/24".to_string()],
                persistent_keepalive_seconds: Some(15),
            }],
        };

        let command = super::build_endpoint_uapi_config(&desired).expect("build endpoint uapi");

        assert_eq!(
            command,
            "set=1\npublic_key=0101010101010101010101010101010101010101010101010101010101010101\nupdate_only=true\nendpoint=10.10.1.10:51820\n\n"
        );
    }

    #[test]
    fn parse_peer_runtime_state_reads_endpoint_and_handshake() {
        let target_key = [7u8; super::WG_KEY_LEN];
        let interface = WireGuardInterface {
            flags: 0,
            listen_port: 51830,
            private_key: [0u8; super::WG_KEY_LEN],
            public_key: [0u8; super::WG_KEY_LEN],
            peers_count: 2,
        };
        let peer_one = WireGuardPeer {
            flags: 0,
            reserved: 0,
            public_key: [1u8; super::WG_KEY_LEN],
            preshared_key: [0u8; super::WG_KEY_LEN],
            persistent_keepalive: 15,
            _endpoint_padding: 0,
            endpoint: [0u8; super::WIREGUARD_SOCKADDR_INET_LEN],
            tx_bytes: 0,
            rx_bytes: 0,
            last_handshake: 0,
            allowed_ips_count: 0,
        };
        let mut endpoint = [0u8; super::WIREGUARD_SOCKADDR_INET_LEN];
        endpoint[0..2].copy_from_slice(&AF_INET.to_ne_bytes());
        endpoint[2..4].copy_from_slice(&34847u16.to_be_bytes());
        endpoint[4..8].copy_from_slice(&[192, 168, 123, 201]);
        let peer_two = WireGuardPeer {
            flags: 0,
            reserved: 0,
            public_key: target_key,
            preshared_key: [0u8; super::WG_KEY_LEN],
            persistent_keepalive: 15,
            _endpoint_padding: 0,
            endpoint,
            tx_bytes: 99,
            rx_bytes: 101,
            last_handshake: 116_444_736_000_000_000 + 42 * 10_000_000,
            allowed_ips_count: 1,
        };
        let allowed_ip = WireGuardAllowedIp {
            flags: 0,
            address: [0u8; 16],
            address_family: AF_INET,
            cidr: 32,
            _padding: 0,
        };

        let mut config = Vec::new();
        config.extend_from_slice(unsafe { any_as_bytes(&interface) });
        config.extend_from_slice(unsafe { any_as_bytes(&peer_one) });
        config.extend_from_slice(unsafe { any_as_bytes(&peer_two) });
        config.extend_from_slice(unsafe { any_as_bytes(&allowed_ip) });

        let state = parse_peer_runtime_state(&config, &target_key)
            .expect("parse runtime state")
            .expect("matching peer");

        assert_eq!(
            state.endpoint,
            Some(Endpoint {
                host: "192.168.123.201".to_string(),
                port: 34847,
            })
        );
        assert_eq!(state.last_handshake_timestamp, 42);
    }

    #[test]
    fn parse_peer_uapi_runtime_state_reads_endpoint_and_handshake() {
        let target_key = [7u8; super::WG_KEY_LEN];
        let response = format!(
            "listen_port=51830\npublic_key={}\nendpoint=192.168.123.201:34847\nlast_handshake_time_sec=42\nrx_bytes=10\ntx_bytes=11\npublic_key={}\nendpoint=10.10.1.10:51820\nlast_handshake_time_sec=7\nerrno=0\n\n",
            hex_key(&target_key),
            hex_key(&[1u8; super::WG_KEY_LEN])
        );

        let state = parse_peer_uapi_runtime_state(&response, &target_key)
            .expect("parse uapi")
            .expect("matching peer");

        assert_eq!(
            state.endpoint,
            Some(Endpoint {
                host: "192.168.123.201".to_string(),
                port: 34847,
            })
        );
        assert_eq!(state.last_handshake_timestamp, 42);
    }

    #[test]
    fn parse_endpoint_value_supports_ipv4_and_bracketed_ipv6() {
        assert_eq!(
            parse_endpoint_value("192.0.2.10:51821").expect("parse ipv4"),
            Endpoint {
                host: "192.0.2.10".to_string(),
                port: 51821,
            }
        );
        assert_eq!(
            parse_endpoint_value("[2001:db8::1]:51821").expect("parse ipv6"),
            Endpoint {
                host: "2001:db8::1".to_string(),
                port: 51821,
            }
        );
    }

    #[test]
    fn filetime_to_unix_seconds_handles_zero_and_epoch() {
        assert_eq!(filetime_to_unix_seconds(0), 0);
        assert_eq!(filetime_to_unix_seconds(116_444_736_000_000_000), 0);
        assert_eq!(
            filetime_to_unix_seconds(116_444_736_000_000_000 + 5 * 10_000_000),
            5
        );
    }

    #[test]
    fn parse_service_state_extracts_running_state() {
        let output = r#"
SERVICE_NAME: WireGuardTunnel$MeshLink
        TYPE               : 10  WIN32_OWN_PROCESS
        STATE              : 4  RUNNING
                                (STOPPABLE, NOT_PAUSABLE, ACCEPTS_SHUTDOWN)
"#;

        assert_eq!(parse_service_state(output), Some("RUNNING"));
    }

    unsafe fn any_as_bytes<T>(value: &T) -> &[u8] {
        std::slice::from_raw_parts((value as *const T).cast::<u8>(), std::mem::size_of::<T>())
    }
}
