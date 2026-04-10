use std::{collections::BTreeSet, fs, path::Path, process::Command};

use anyhow::{anyhow, bail, Context, Result};
use tempfile::NamedTempFile;
use wg_manager::{DesiredState, WireGuardBackend};

const IP_BIN_PRIMARY: &str = "/usr/sbin/ip";
const WG_BIN_PRIMARY: &str = "/usr/bin/wg";

#[derive(Debug, Default, Clone, Copy)]
pub struct LinuxWireGuardBackend;

impl LinuxWireGuardBackend {
    pub fn new() -> Self {
        Self
    }
}

impl WireGuardBackend for LinuxWireGuardBackend {
    fn reconcile(&self, desired: &DesiredState) -> Result<()> {
        let ip_bin = resolve_binary(IP_BIN_PRIMARY, "ip");
        let wg_bin = resolve_binary(WG_BIN_PRIMARY, "wg");

        ensure_interface(&ip_bin, &desired.interface_name)?;
        sync_interface_address(&ip_bin, &desired.interface_name, &desired.address_cidr)?;
        sync_wireguard_config(&wg_bin, desired)?;
        run_checked(
            &ip_bin,
            &["link", "set", "up", "dev", desired.interface_name.as_str()],
        )?;
        sync_routes(
            &ip_bin,
            &desired.interface_name,
            &desired.route_destinations(),
        )?;
        Ok(())
    }
}

fn ensure_interface(ip_bin: &str, interface_name: &str) -> Result<()> {
    if run(ip_bin, &["link", "show", "dev", interface_name])?
        .status
        .success()
    {
        return Ok(());
    }

    run_checked(
        ip_bin,
        &["link", "add", "dev", interface_name, "type", "wireguard"],
    )
}

fn sync_interface_address(ip_bin: &str, interface_name: &str, address_cidr: &str) -> Result<()> {
    let current = run_checked_output(
        ip_bin,
        &["-o", "-4", "address", "show", "dev", interface_name],
    )?;
    let current_stdout = String::from_utf8_lossy(&current.stdout);
    for cidr in parse_interface_addresses(&current_stdout) {
        if cidr != address_cidr {
            run_checked(
                ip_bin,
                &["address", "del", cidr.as_str(), "dev", interface_name],
            )?;
        }
    }

    run_checked(
        ip_bin,
        &["address", "replace", address_cidr, "dev", interface_name],
    )
}

fn sync_wireguard_config(wg_bin: &str, desired: &DesiredState) -> Result<()> {
    let temp_file = NamedTempFile::new().context("create temporary wireguard config")?;
    fs::write(temp_file.path(), render_sync_config(desired)).with_context(|| {
        format!(
            "write temporary wireguard config {}",
            temp_file.path().display()
        )
    })?;

    run_checked(
        wg_bin,
        &[
            "syncconf",
            desired.interface_name.as_str(),
            temp_file
                .path()
                .to_str()
                .ok_or_else(|| anyhow!("temporary config path is not valid UTF-8"))?,
        ],
    )
}

fn sync_routes(ip_bin: &str, interface_name: &str, desired_routes: &[String]) -> Result<()> {
    let desired_set = desired_routes.iter().cloned().collect::<BTreeSet<_>>();
    let current = run_checked_output(ip_bin, &["-o", "route", "show", "dev", interface_name])?;
    let current_stdout = String::from_utf8_lossy(&current.stdout);
    let current_set = parse_route_destinations(&current_stdout);

    for route in current_set.difference(&desired_set) {
        run_checked(
            ip_bin,
            &["route", "del", route.as_str(), "dev", interface_name],
        )?;
    }
    for route in desired_set {
        run_checked(
            ip_bin,
            &["route", "replace", route.as_str(), "dev", interface_name],
        )?;
    }

    Ok(())
}

fn render_sync_config(desired: &DesiredState) -> String {
    let mut rendered = format!(
        "[Interface]\nPrivateKey = {}\nListenPort = {}\n",
        desired.private_key, desired.listen_port
    );

    for peer in &desired.peers {
        rendered.push_str("\n[Peer]\n");
        rendered.push_str(&format!("PublicKey = {}\n", peer.public_key));
        rendered.push_str(&format!("Endpoint = {}\n", peer.endpoint.render()));
        rendered.push_str(&format!("AllowedIPs = {}\n", peer.allowed_ips.join(", ")));
    }

    rendered
}

fn parse_interface_addresses(output: &str) -> BTreeSet<String> {
    output
        .lines()
        .filter_map(|line| line.split_whitespace().nth(3))
        .map(ToString::to_string)
        .collect()
}

fn parse_route_destinations(output: &str) -> BTreeSet<String> {
    output
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|token| *token != "default")
        .map(ToString::to_string)
        .collect()
}

fn resolve_binary(primary: &str, fallback: &str) -> String {
    if Path::new(primary).exists() {
        primary.to_string()
    } else {
        fallback.to_string()
    }
}

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

fn run_checked_output(program: &str, args: &[&str]) -> Result<std::process::Output> {
    let output = run(program, args)?;
    if output.status.success() {
        return Ok(output);
    }

    bail!(
        "command failed: {} {}: {}",
        program,
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

fn run(program: &str, args: &[&str]) -> Result<std::process::Output> {
    Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("run {} {}", program, args.join(" ")))
}

#[cfg(test)]
mod tests {
    use super::{parse_interface_addresses, parse_route_destinations, render_sync_config};
    use wg_manager::{DesiredPeer, DesiredState, Endpoint};

    #[test]
    fn render_sync_config_includes_interface_and_peers() {
        let rendered = render_sync_config(&DesiredState {
            interface_name: "sdwan0".to_string(),
            private_key: "private-key".to_string(),
            listen_port: 51820,
            address_cidr: "100.64.0.1/32".to_string(),
            peers: vec![DesiredPeer {
                peer_id: "dev-b".to_string(),
                public_key: "pk-b".to_string(),
                endpoint: Endpoint {
                    host: "192.0.2.20".to_string(),
                    port: 51821,
                },
                allowed_ips: vec!["100.64.0.2/32".to_string()],
            }],
        });

        assert!(rendered.contains("PrivateKey = private-key"));
        assert!(rendered.contains("ListenPort = 51820"));
        assert!(rendered.contains("Endpoint = 192.0.2.20:51821"));
        assert!(rendered.contains("AllowedIPs = 100.64.0.2/32"));
    }

    #[test]
    fn parse_interface_addresses_extracts_cidrs() {
        let output = "8: sdwan0    inet 100.64.0.1/32 scope global sdwan0\n";
        let addresses = parse_interface_addresses(output);

        assert!(addresses.contains("100.64.0.1/32"));
    }

    #[test]
    fn parse_route_destinations_extracts_route_targets() {
        let output = "100.64.0.2/32 dev sdwan0 scope link\n100.64.0.3/32 dev sdwan0 scope link\n";
        let routes = parse_route_destinations(output);

        assert!(routes.contains("100.64.0.2/32"));
        assert!(routes.contains("100.64.0.3/32"));
    }
}
