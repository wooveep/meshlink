use std::{path::{Path, PathBuf}, process::Command};

#[cfg(windows)]
use std::{ffi::OsStr, fs, os::windows::ffi::OsStrExt};

use anyhow::{bail, Context, Result};
use wg_manager::{DesiredState, WireGuardBackend};

#[cfg_attr(not(windows), allow(dead_code))]
const WINDOWS_RUNTIME_VERSION: &str = "v0.3.17";

#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsWireGuardBackend;

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
    fs::write(&config_path, render_tunnel_config(desired))
        .with_context(|| format!("write windows wireguard config {}", config_path.display()))?;

    ensure_runtime_assets_present()?;
    ensure_tunnel_service(desired, &config_path)
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
        let service: libloading::Symbol<
            unsafe extern "system" fn(*const u16) -> u32,
        > = library
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
fn ensure_tunnel_service(desired: &DesiredState, config_path: &Path) -> Result<()> {
    let service_name = service_name(&desired.interface_name);
    let display_name = display_name(&desired.interface_name);
    let binary = std::env::current_exe().context("resolve meshlinkd.exe path")?;
    let bin_path = service_bin_path(&binary, config_path);
    let sc = resolve_sc_exe();

    if service_exists(&service_name)? {
        let _ = run(&sc, &["stop", service_name.as_str()]);
        run_checked(
            &sc,
            &[
                "config",
                service_name.as_str(),
                "binPath=",
                bin_path.as_str(),
                "start=",
                "auto",
                "displayname=",
                display_name.as_str(),
            ],
        )?;
    } else {
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
    }

    run_checked(&sc, &["sidtype", service_name.as_str(), "unrestricted"])?;
    let _ = run(&sc, &["start", service_name.as_str()]);
    Ok(())
}

#[cfg(windows)]
fn service_exists(service_name: &str) -> Result<bool> {
    let output = run(&resolve_sc_exe(), &["query", service_name])?;
    Ok(output.status.success())
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
fn ensure_runtime_assets_present() -> Result<()> {
    resolve_runtime_asset("tunnel.dll")?;
    resolve_runtime_asset("wireguard.dll")?;
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
fn run(program: &str, args: &[&str]) -> Result<std::process::Output> {
    Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("run {} {}", program, args.join(" ")))
}

#[cfg(test)]
mod tests {
    use super::{display_name, render_tunnel_config, service_name, stable_config_path};
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
}
