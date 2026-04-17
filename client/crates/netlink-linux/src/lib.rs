use std::{collections::BTreeSet, path::Path, process::Command};

use anyhow::{bail, Context, Result};
use wg_manager::{DesiredState, WireGuardBackend};

const IP_BIN_PRIMARY: &str = "/usr/sbin/ip";

#[derive(Debug, Default, Clone, Copy)]
pub struct LinuxWireGuardBackend;

impl LinuxWireGuardBackend {
    pub fn new() -> Self {
        Self
    }
}

impl WireGuardBackend for LinuxWireGuardBackend {
    fn reconcile(&self, desired: &DesiredState) -> Result<()> {
        reconcile_linux(desired)
    }
}

pub fn latest_handshake_timestamp(interface_name: &str, peer_public_key: &str) -> Result<u64> {
    latest_handshake_linux(interface_name, peer_public_key)
}

#[cfg(target_os = "linux")]
fn reconcile_linux(desired: &DesiredState) -> Result<()> {
    let ip_bin = resolve_binary(IP_BIN_PRIMARY, "ip");

    ensure_interface(&ip_bin, &desired.interface_name)?;
    sync_interface_address(&ip_bin, &desired.interface_name, &desired.address_cidr)?;
    wireguard_uapi::apply_config(desired)?;
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

#[cfg(not(target_os = "linux"))]
fn reconcile_linux(_desired: &DesiredState) -> Result<()> {
    bail!("linux wireguard backend is only available on Linux")
}

#[cfg(target_os = "linux")]
fn latest_handshake_linux(interface_name: &str, peer_public_key: &str) -> Result<u64> {
    wireguard_uapi::latest_handshake_timestamp(interface_name, peer_public_key)
}

#[cfg(not(target_os = "linux"))]
fn latest_handshake_linux(_interface_name: &str, _peer_public_key: &str) -> Result<u64> {
    Ok(0)
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

#[cfg(target_os = "linux")]
mod wireguard_uapi {
    use std::{
        ffi::{CStr, CString},
        io, mem,
        net::IpAddr,
        os::raw::{c_char, c_int},
        ptr,
        str::FromStr,
    };

    use anyhow::{anyhow, bail, Context, Result};
    use libc::{
        c_uint, in6_addr, in_addr, sockaddr, sockaddr_in, sockaddr_in6, AF_INET, AF_INET6, IFNAMSIZ,
    };
    use wg_manager::{DesiredPeer, DesiredState};

    const WG_KEY_LEN: usize = 32;
    const WG_KEY_B64_LEN: usize = 45;

    const WGDEVICE_REPLACE_PEERS: c_uint = 1 << 0;
    const WGDEVICE_HAS_PRIVATE_KEY: c_uint = 1 << 1;
    const WGDEVICE_HAS_LISTEN_PORT: c_uint = 1 << 3;

    const WGPEER_REPLACE_ALLOWEDIPS: c_uint = 1 << 1;
    const WGPEER_HAS_PUBLIC_KEY: c_uint = 1 << 2;
    const WGPEER_HAS_PERSISTENT_KEEPALIVE_INTERVAL: c_uint = 1 << 4;

    #[repr(C)]
    struct timespec64 {
        tv_sec: i64,
        tv_nsec: i64,
    }

    #[repr(C)]
    union wg_allowedip_addr {
        ip4: in_addr,
        ip6: in6_addr,
    }

    #[repr(C)]
    struct wg_allowedip {
        family: u16,
        ip: wg_allowedip_addr,
        cidr: u8,
        next_allowedip: *mut wg_allowedip,
    }

    #[repr(C)]
    union wg_endpoint {
        addr: sockaddr,
        addr4: sockaddr_in,
        addr6: sockaddr_in6,
    }

    #[repr(C)]
    struct wg_peer {
        flags: c_uint,
        public_key: [u8; WG_KEY_LEN],
        preshared_key: [u8; WG_KEY_LEN],
        endpoint: wg_endpoint,
        last_handshake_time: timespec64,
        rx_bytes: u64,
        tx_bytes: u64,
        persistent_keepalive_interval: u16,
        first_allowedip: *mut wg_allowedip,
        last_allowedip: *mut wg_allowedip,
        next_peer: *mut wg_peer,
    }

    #[repr(C)]
    struct wg_device {
        name: [c_char; IFNAMSIZ],
        ifindex: u32,
        flags: c_uint,
        public_key: [u8; WG_KEY_LEN],
        private_key: [u8; WG_KEY_LEN],
        fwmark: u32,
        listen_port: u16,
        first_peer: *mut wg_peer,
        last_peer: *mut wg_peer,
    }

    unsafe extern "C" {
        fn wg_set_device(dev: *mut wg_device) -> c_int;
        fn wg_get_device(dev: *mut *mut wg_device, device_name: *const c_char) -> c_int;
        fn wg_free_device(dev: *mut wg_device);
        fn wg_key_to_base64(base64: *mut c_char, key: *const u8);
        fn wg_key_from_base64(key: *mut u8, base64: *const c_char) -> c_int;
    }

    pub fn apply_config(desired: &DesiredState) -> Result<()> {
        let mut allowedip_storage = Vec::<Box<wg_allowedip>>::new();
        let mut peers = Vec::<Box<wg_peer>>::new();

        for desired_peer in &desired.peers {
            let mut peer_allowedips = build_allowedips(&desired_peer.allowed_ips)
                .with_context(|| format!("build allowed IPs for peer {}", desired_peer.peer_id))?;
            link_allowedips(&mut peer_allowedips);

            let first_allowedip = peer_allowedips
                .first_mut()
                .map(|item| item.as_mut() as *mut wg_allowedip)
                .unwrap_or(ptr::null_mut());
            let last_allowedip = peer_allowedips
                .last_mut()
                .map(|item| item.as_mut() as *mut wg_allowedip)
                .unwrap_or(ptr::null_mut());

            allowedip_storage.extend(peer_allowedips.into_iter());

            let mut peer = zeroed_peer();
            peer.flags = WGPEER_HAS_PUBLIC_KEY | WGPEER_REPLACE_ALLOWEDIPS;
            peer.public_key = decode_key(&desired_peer.public_key)
                .with_context(|| format!("decode public key for peer {}", desired_peer.peer_id))?;
            peer.endpoint = build_endpoint(desired_peer)
                .with_context(|| format!("build endpoint for peer {}", desired_peer.peer_id))?;
            peer.first_allowedip = first_allowedip;
            peer.last_allowedip = last_allowedip;

            if let Some(keepalive) = desired_peer.persistent_keepalive_seconds {
                peer.flags |= WGPEER_HAS_PERSISTENT_KEEPALIVE_INTERVAL;
                peer.persistent_keepalive_interval = keepalive;
            }

            peers.push(Box::new(peer));
        }

        link_peers(&mut peers);

        let mut device = zeroed_device();
        set_device_name(&mut device.name, &desired.interface_name)?;
        device.flags = WGDEVICE_REPLACE_PEERS | WGDEVICE_HAS_PRIVATE_KEY | WGDEVICE_HAS_LISTEN_PORT;
        device.private_key =
            decode_key(&desired.private_key).context("decode interface private key")?;
        device.listen_port = desired.listen_port;
        device.first_peer = peers
            .first_mut()
            .map(|peer| peer.as_mut() as *mut wg_peer)
            .unwrap_or(ptr::null_mut());
        device.last_peer = peers
            .last_mut()
            .map(|peer| peer.as_mut() as *mut wg_peer)
            .unwrap_or(ptr::null_mut());

        let rc = unsafe { wg_set_device(&mut device) };
        if rc == 0 {
            return Ok(());
        }

        Err(io::Error::last_os_error())
            .context("apply wireguard configuration through embeddable UAPI")
    }

    pub fn latest_handshake_timestamp(interface_name: &str, peer_public_key: &str) -> Result<u64> {
        let interface_name = CString::new(interface_name).context("encode interface name")?;
        let mut device_ptr = ptr::null_mut::<wg_device>();
        let rc = unsafe { wg_get_device(&mut device_ptr, interface_name.as_ptr()) };
        if rc != 0 {
            let err = io::Error::last_os_error();
            return match err.raw_os_error() {
                Some(libc::ENODEV) | Some(libc::ENOENT) => Ok(0),
                _ => Err(err).context("read wireguard device state through embeddable UAPI"),
            };
        }
        if device_ptr.is_null() {
            return Ok(0);
        }

        let device = DeviceGuard(device_ptr);
        let mut peer = unsafe { (*device.0).first_peer };
        while !peer.is_null() {
            if encode_key(unsafe { &(*peer).public_key })? == peer_public_key {
                return Ok(unsafe { (*peer).last_handshake_time.tv_sec.max(0) as u64 });
            }
            peer = unsafe { (*peer).next_peer };
        }

        Ok(0)
    }

    fn zeroed_peer() -> wg_peer {
        unsafe { mem::zeroed() }
    }

    fn zeroed_device() -> wg_device {
        unsafe { mem::zeroed() }
    }

    fn set_device_name(dst: &mut [c_char; IFNAMSIZ], interface_name: &str) -> Result<()> {
        let raw = interface_name.as_bytes();
        if raw.is_empty() {
            bail!("interface name cannot be empty");
        }
        if raw.len() >= IFNAMSIZ {
            bail!(
                "interface name '{}' is too long for IFNAMSIZ",
                interface_name
            );
        }

        for (idx, byte) in raw.iter().enumerate() {
            dst[idx] = *byte as c_char;
        }
        Ok(())
    }

    fn decode_key(base64: &str) -> Result<[u8; WG_KEY_LEN]> {
        let encoded = CString::new(base64.trim()).context("encode base64 key")?;
        let mut key = [0u8; WG_KEY_LEN];
        let rc = unsafe { wg_key_from_base64(key.as_mut_ptr(), encoded.as_ptr()) };
        if rc == 0 {
            Ok(key)
        } else {
            bail!("invalid wireguard key")
        }
    }

    fn encode_key(key: &[u8; WG_KEY_LEN]) -> Result<String> {
        let mut output = [0 as c_char; WG_KEY_B64_LEN];
        unsafe {
            wg_key_to_base64(output.as_mut_ptr(), key.as_ptr());
            Ok(CStr::from_ptr(output.as_ptr())
                .to_str()
                .context("decode base64 public key")?
                .to_string())
        }
    }

    fn build_endpoint(desired_peer: &DesiredPeer) -> Result<wg_endpoint> {
        let ip = IpAddr::from_str(desired_peer.endpoint.host.trim()).with_context(|| {
            format!("parse endpoint IP '{}'", desired_peer.endpoint.host.trim())
        })?;

        Ok(match ip {
            IpAddr::V4(addr) => wg_endpoint {
                addr4: sockaddr_in {
                    sin_family: AF_INET as u16,
                    sin_port: desired_peer.endpoint.port.to_be(),
                    sin_addr: in_addr {
                        s_addr: u32::from_ne_bytes(addr.octets()),
                    },
                    sin_zero: [0; 8],
                },
            },
            IpAddr::V6(addr) => wg_endpoint {
                addr6: sockaddr_in6 {
                    sin6_family: AF_INET6 as u16,
                    sin6_port: desired_peer.endpoint.port.to_be(),
                    sin6_flowinfo: 0,
                    sin6_addr: in6_addr {
                        s6_addr: addr.octets(),
                    },
                    sin6_scope_id: 0,
                },
            },
        })
    }

    fn build_allowedips(allowed_ips: &[String]) -> Result<Vec<Box<wg_allowedip>>> {
        let mut built = Vec::with_capacity(allowed_ips.len());
        for cidr in allowed_ips {
            built.push(Box::new(build_allowedip(cidr)?));
        }
        Ok(built)
    }

    fn build_allowedip(cidr: &str) -> Result<wg_allowedip> {
        let (address, prefix) = cidr
            .trim()
            .split_once('/')
            .ok_or_else(|| anyhow!("CIDR '{}' is missing prefix length", cidr.trim()))?;
        let prefix = prefix
            .parse::<u8>()
            .with_context(|| format!("parse prefix length '{}'", prefix))?;
        let ip =
            IpAddr::from_str(address).with_context(|| format!("parse allowed IP '{}'", address))?;

        Ok(match ip {
            IpAddr::V4(addr) => {
                if prefix > 32 {
                    bail!("IPv4 prefix length {} is out of range", prefix);
                }
                wg_allowedip {
                    family: AF_INET as u16,
                    ip: wg_allowedip_addr {
                        ip4: in_addr {
                            s_addr: u32::from_ne_bytes(addr.octets()),
                        },
                    },
                    cidr: prefix,
                    next_allowedip: ptr::null_mut(),
                }
            }
            IpAddr::V6(addr) => {
                if prefix > 128 {
                    bail!("IPv6 prefix length {} is out of range", prefix);
                }
                wg_allowedip {
                    family: AF_INET6 as u16,
                    ip: wg_allowedip_addr {
                        ip6: in6_addr {
                            s6_addr: addr.octets(),
                        },
                    },
                    cidr: prefix,
                    next_allowedip: ptr::null_mut(),
                }
            }
        })
    }

    fn link_allowedips(allowedips: &mut [Box<wg_allowedip>]) {
        for idx in 0..allowedips.len().saturating_sub(1) {
            let next = allowedips[idx + 1].as_mut() as *mut wg_allowedip;
            allowedips[idx].next_allowedip = next;
        }
    }

    fn link_peers(peers: &mut [Box<wg_peer>]) {
        for idx in 0..peers.len().saturating_sub(1) {
            let next = peers[idx + 1].as_mut() as *mut wg_peer;
            peers[idx].next_peer = next;
        }
    }

    struct DeviceGuard(*mut wg_device);

    impl Drop for DeviceGuard {
        fn drop(&mut self) {
            unsafe { wg_free_device(self.0) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_interface_addresses, parse_route_destinations};
    use std::collections::BTreeSet;

    #[test]
    fn parse_interface_addresses_ignores_other_columns() {
        let parsed = parse_interface_addresses(
            "7: meshlink0    inet 100.64.0.1/32 brd 100.64.0.1 scope global meshlink0\n",
        );

        assert_eq!(parsed, BTreeSet::from(["100.64.0.1/32".to_string()]));
    }

    #[test]
    fn parse_route_destinations_ignores_default_route() {
        let parsed = parse_route_destinations(
            "default via 192.0.2.1 dev eth0\n100.64.0.2 proto static scope link\n10.20.0.0/24 proto static scope link\n",
        );

        assert_eq!(
            parsed,
            BTreeSet::from(["10.20.0.0/24".to_string(), "100.64.0.2".to_string()])
        );
    }
}
