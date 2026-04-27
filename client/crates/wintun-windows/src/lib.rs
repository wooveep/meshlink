use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(windows)]
use std::{ffi::OsStr, fs, io, os::windows::ffi::OsStrExt};

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
    endpoint: [u8; WIREGUARD_SOCKADDR_INET_LEN],
    tx_bytes: u64,
    rx_bytes: u64,
    last_handshake: u64,
    allowed_ips_count: u32,
}

#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct WireGuardAllowedIp {
    address: [u8; 16],
    address_family: u16,
    cidr: u8,
    _padding: u8,
    flags: u32,
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
    let config_changed =
        write_tunnel_config_if_changed(&config_path, &render_tunnel_config(desired))?;

    ensure_runtime_assets_present()?;
    ensure_tunnel_service(desired, &config_path, config_changed)
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
    let api = WireGuardApi::load()?;
    let adapter = match api.open_adapter(interface_name)? {
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
    config_changed: bool,
) -> Result<()> {
    let service_name = service_name(&desired.interface_name);
    let display_name = display_name(&desired.interface_name);
    let binary = std::env::current_exe().context("resolve meshlinkd.exe path")?;
    let bin_path = service_bin_path(&binary, config_path);
    let sc = resolve_sc_exe();

    if !service_exists(&service_name)? {
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
        run_checked(&sc, &["stop", service_name.as_str()])?;
        wait_for_service_state(&service_name, "STOPPED")?;
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
fn service_bin_path(binary: &Path, config_path: &Path) -> String {
    format!(
        "\"{}\" /service \"{}\"",
        binary.display(),
        config_path.display()
    )
}

#[cfg(windows)]
fn write_tunnel_config_if_changed(path: &Path, rendered: &str) -> Result<bool> {
    if matches!(fs::read_to_string(path), Ok(existing) if existing == rendered) {
        return Ok(false);
    }

    fs::write(path, rendered)
        .with_context(|| format!("write windows wireguard config {}", path.display()))?;
    Ok(true)
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
struct WireGuardApi {
    _library: libloading::Library,
    open_adapter: WireGuardOpenAdapterFn,
    close_adapter: WireGuardCloseAdapterFn,
    get_configuration: WireGuardGetConfigurationFn,
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
            Ok(Self {
                _library: library,
                open_adapter,
                close_adapter,
                get_configuration,
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
}

fn decode_public_key(public_key: &str) -> Result<[u8; WG_KEY_LEN]> {
    let decoded = BASE64
        .decode(public_key.trim())
        .context("decode base64 wireguard public key")?;
    let key: [u8; WG_KEY_LEN] = decoded
        .try_into()
        .map_err(|_| anyhow!("wireguard public key must be {} bytes", WG_KEY_LEN))?;
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
        display_name, endpoint_from_raw, filetime_to_unix_seconds, parse_peer_runtime_state,
        parse_service_state, render_tunnel_config, service_name, stable_config_path,
        WireGuardAllowedIp, WireGuardInterface, WireGuardPeer, AF_INET,
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
            endpoint,
            tx_bytes: 99,
            rx_bytes: 101,
            last_handshake: 116_444_736_000_000_000 + 42 * 10_000_000,
            allowed_ips_count: 1,
        };
        let allowed_ip = WireGuardAllowedIp {
            address: [0u8; 16],
            address_family: AF_INET,
            cidr: 32,
            _padding: 0,
            flags: 0,
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
