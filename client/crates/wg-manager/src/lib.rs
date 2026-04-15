use std::{collections::BTreeMap, fmt};

use anyhow::{anyhow, Result};
use api_client::proto::{Device, Peer};

pub trait WireGuardBackend {
    fn reconcile(&self, desired: &DesiredState) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredState {
    pub interface_name: String,
    pub private_key: String,
    pub listen_port: u16,
    pub address_cidr: String,
    pub peers: Vec<DesiredPeer>,
}

impl DesiredState {
    pub fn route_destinations(&self) -> Vec<String> {
        self.peers
            .iter()
            .flat_map(|peer| peer.allowed_ips.iter().cloned())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredPeer {
    pub peer_id: String,
    pub public_key: String,
    pub endpoint: Endpoint,
    pub allowed_ips: Vec<String>,
    pub persistent_keepalive_seconds: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
}

impl Endpoint {
    pub fn render(&self) -> String {
        if self.host.contains(':') && !self.host.starts_with('[') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildOutcome {
    pub desired_state: DesiredState,
    pub skipped_peers: Vec<SkippedPeer>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedPeer {
    pub peer_id: String,
    pub reason: SkipReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    MissingDirectEndpoint,
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SkipReason::MissingDirectEndpoint => f.write_str("missing_direct_endpoint"),
        }
    }
}

pub fn build_desired_state(
    interface_name: &str,
    private_key: &str,
    listen_port: u16,
    self_device: &Device,
    peers: &[Peer],
) -> Result<BuildOutcome> {
    build_desired_state_with_overrides(
        interface_name,
        private_key,
        listen_port,
        self_device,
        peers,
        &BTreeMap::new(),
    )
}

pub fn build_desired_state_with_overrides(
    interface_name: &str,
    private_key: &str,
    listen_port: u16,
    self_device: &Device,
    peers: &[Peer],
    endpoint_overrides: &BTreeMap<String, Endpoint>,
) -> Result<BuildOutcome> {
    let overlay_ipv4 = self_device
        .overlay
        .as_ref()
        .map(|overlay| overlay.ipv4.trim())
        .filter(|ipv4| !ipv4.is_empty())
        .ok_or_else(|| anyhow!("self device is missing overlay IPv4"))?;

    let mut desired_peers = Vec::new();
    let mut skipped_peers = Vec::new();

    for peer in peers {
        let peer_id = peer.peer_id.clone();
        let endpoint = if let Some(override_endpoint) = endpoint_overrides.get(&peer_id) {
            override_endpoint.clone()
        } else {
            let Some(endpoint) = peer.direct_endpoint.as_ref() else {
                skipped_peers.push(SkippedPeer {
                    peer_id,
                    reason: SkipReason::MissingDirectEndpoint,
                });
                continue;
            };

            Endpoint {
                host: endpoint.host.clone(),
                port: u16::try_from(endpoint.port)
                    .map_err(|_| anyhow!("peer {} endpoint port out of range", peer.peer_id))?,
            }
        };

        let allowed_ips = if peer.allowed_ips.is_empty() {
            peer.overlay
                .as_ref()
                .map(|overlay| overlay.ipv4.trim())
                .filter(|ipv4| !ipv4.is_empty())
                .map(|ipv4| vec![format!("{ipv4}/32")])
                .unwrap_or_default()
        } else {
            peer.allowed_ips.clone()
        };

        desired_peers.push(DesiredPeer {
            peer_id: peer.peer_id.clone(),
            public_key: peer.public_key.clone(),
            endpoint,
            allowed_ips,
            persistent_keepalive_seconds: Some(15),
        });
    }

    desired_peers.sort_by(|left, right| left.peer_id.cmp(&right.peer_id));

    Ok(BuildOutcome {
        desired_state: DesiredState {
            interface_name: interface_name.to_string(),
            private_key: private_key.to_string(),
            listen_port,
            address_cidr: format!("{overlay_ipv4}/32"),
            peers: desired_peers,
        },
        skipped_peers,
    })
}

#[cfg(test)]
mod tests {
    use super::{build_desired_state, build_desired_state_with_overrides, Endpoint, SkipReason};
    use api_client::proto::{Device, DirectEndpoint, OverlayAddress, Peer};
    use std::collections::BTreeMap;

    #[test]
    fn build_desired_state_uses_overlay_and_endpoint() {
        let outcome = build_desired_state(
            "sdwan0",
            "private-key",
            51820,
            &Device {
                id: "dev-a".to_string(),
                overlay: Some(OverlayAddress {
                    ipv4: "100.64.0.1".to_string(),
                    ipv6: String::new(),
                }),
                ..Default::default()
            },
            &[Peer {
                peer_id: "dev-b".to_string(),
                public_key: "pk-b".to_string(),
                overlay: Some(OverlayAddress {
                    ipv4: "100.64.0.2".to_string(),
                    ipv6: String::new(),
                }),
                allowed_ips: vec!["100.64.0.2/32".to_string()],
                direct_endpoint: Some(DirectEndpoint {
                    host: "192.0.2.20".to_string(),
                    port: 51821,
                }),
                ..Default::default()
            }],
        )
        .expect("build desired state");

        assert_eq!(outcome.desired_state.address_cidr, "100.64.0.1/32");
        assert_eq!(outcome.desired_state.peers.len(), 1);
        assert_eq!(
            outcome.desired_state.peers[0].endpoint,
            Endpoint {
                host: "192.0.2.20".to_string(),
                port: 51821,
            }
        );
        assert_eq!(
            outcome.desired_state.peers[0].persistent_keepalive_seconds,
            Some(15)
        );
        assert!(outcome.skipped_peers.is_empty());
    }

    #[test]
    fn build_desired_state_skips_peers_without_direct_endpoint() {
        let outcome = build_desired_state(
            "sdwan0",
            "private-key",
            51820,
            &Device {
                id: "dev-a".to_string(),
                overlay: Some(OverlayAddress {
                    ipv4: "100.64.0.1".to_string(),
                    ipv6: String::new(),
                }),
                ..Default::default()
            },
            &[Peer {
                peer_id: "dev-b".to_string(),
                public_key: "pk-b".to_string(),
                allowed_ips: vec!["100.64.0.2/32".to_string()],
                ..Default::default()
            }],
        )
        .expect("build desired state");

        assert!(outcome.desired_state.peers.is_empty());
        assert_eq!(outcome.skipped_peers.len(), 1);
        assert_eq!(outcome.skipped_peers[0].peer_id, "dev-b");
        assert_eq!(
            outcome.skipped_peers[0].reason,
            SkipReason::MissingDirectEndpoint
        );
    }

    #[test]
    fn build_desired_state_preserves_extra_allowed_ips() {
        let outcome = build_desired_state(
            "sdwan0",
            "private-key",
            51820,
            &Device {
                id: "dev-a".to_string(),
                overlay: Some(OverlayAddress {
                    ipv4: "100.64.0.1".to_string(),
                    ipv6: String::new(),
                }),
                ..Default::default()
            },
            &[Peer {
                peer_id: "dev-b".to_string(),
                public_key: "pk-b".to_string(),
                allowed_ips: vec!["100.64.0.2/32".to_string(), "10.20.0.0/24".to_string()],
                direct_endpoint: Some(DirectEndpoint {
                    host: "192.0.2.20".to_string(),
                    port: 51821,
                }),
                ..Default::default()
            }],
        )
        .expect("build desired state");

        assert_eq!(
            outcome.desired_state.peers[0].allowed_ips,
            vec!["100.64.0.2/32".to_string(), "10.20.0.0/24".to_string()]
        );
    }

    #[test]
    fn endpoint_render_brackets_ipv6() {
        let endpoint = Endpoint {
            host: "2001:db8::10".to_string(),
            port: 51820,
        };

        assert_eq!(endpoint.render(), "[2001:db8::10]:51820");
    }

    #[test]
    fn build_desired_state_prefers_runtime_endpoint_override() {
        let mut overrides = BTreeMap::new();
        overrides.insert(
            "dev-b".to_string(),
            Endpoint {
                host: "203.0.113.20".to_string(),
                port: 60000,
            },
        );

        let outcome = build_desired_state_with_overrides(
            "sdwan0",
            "private-key",
            51820,
            &Device {
                id: "dev-a".to_string(),
                overlay: Some(OverlayAddress {
                    ipv4: "100.64.0.1".to_string(),
                    ipv6: String::new(),
                }),
                ..Default::default()
            },
            &[Peer {
                peer_id: "dev-b".to_string(),
                public_key: "pk-b".to_string(),
                allowed_ips: vec!["100.64.0.2/32".to_string()],
                direct_endpoint: Some(DirectEndpoint {
                    host: "192.0.2.20".to_string(),
                    port: 51821,
                }),
                ..Default::default()
            }],
            &overrides,
        )
        .expect("build desired state with override");

        assert_eq!(
            outcome.desired_state.peers[0].endpoint,
            Endpoint {
                host: "203.0.113.20".to_string(),
                port: 60000,
            }
        );
    }
}
