use std::{
    collections::{BTreeMap, BTreeSet},
    net::{IpAddr, SocketAddr},
    sync::Arc,
};

use anyhow::{Context, Result};
use tokio::{
    net::UdpSocket,
    sync::{mpsc, Mutex},
    task::JoinHandle,
};
use tracing::warn;
use wg_manager::Endpoint;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedRemoteEndpoint {
    pub peer_id: String,
    pub endpoint: Endpoint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyPeerHandle {
    pub peer_id: String,
    pub loopback_endpoint: Endpoint,
}

#[derive(Debug)]
pub struct PublicUdpProxy {
    public_socket: Arc<UdpSocket>,
    state: Arc<Mutex<ProxyState>>,
    observation_rx: Option<mpsc::Receiver<ObservedRemoteEndpoint>>,
    public_task: Mutex<Option<JoinHandle<()>>>,
    kernel_listen_port: u16,
}

#[derive(Debug)]
struct ProxyState {
    kernel_wg_addr: SocketAddr,
    observation_tx: mpsc::Sender<ObservedRemoteEndpoint>,
    peers: BTreeMap<String, ProxyPeerState>,
}

#[derive(Debug)]
struct ProxyPeerState {
    loopback_socket: Arc<UdpSocket>,
    loopback_addr: SocketAddr,
    outbound_task: JoinHandle<()>,
    remote_endpoint: Option<Endpoint>,
}

#[derive(Debug)]
struct InboundPeerMatch {
    recipients: Vec<Arc<UdpSocket>>,
    observed: Option<ObservedRemoteEndpoint>,
}

impl PublicUdpProxy {
    pub async fn bind(public_port: u16) -> Result<Self> {
        let public_socket = Arc::new(
            UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], public_port)))
                .await
                .with_context(|| {
                    format!("bind public udp proxy socket on 0.0.0.0:{public_port}")
                })?,
        );
        let kernel_probe = UdpSocket::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .context("allocate kernel wireguard listen port for proxy mode")?;
        let kernel_listen_port = kernel_probe
            .local_addr()
            .context("read allocated kernel listen port")?
            .port();
        drop(kernel_probe);

        let (observation_tx, observation_rx) = mpsc::channel(64);
        let state = Arc::new(Mutex::new(ProxyState {
            kernel_wg_addr: SocketAddr::from(([127, 0, 0, 1], kernel_listen_port)),
            observation_tx,
            peers: BTreeMap::new(),
        }));
        Ok(Self {
            public_socket,
            state,
            observation_rx: Some(observation_rx),
            public_task: Mutex::new(None),
            kernel_listen_port,
        })
    }

    pub fn public_socket(&self) -> Arc<UdpSocket> {
        self.public_socket.clone()
    }

    pub fn public_addr(&self) -> Result<SocketAddr> {
        self.public_socket
            .local_addr()
            .context("read public udp proxy address")
    }

    pub fn kernel_listen_port(&self) -> u16 {
        self.kernel_listen_port
    }

    pub fn take_observation_rx(&mut self) -> Option<mpsc::Receiver<ObservedRemoteEndpoint>> {
        self.observation_rx.take()
    }

    pub async fn ensure_peer(&self, peer_id: &str) -> Result<ProxyPeerHandle> {
        self.ensure_started().await;
        if let Some(existing) = self.local_endpoint(peer_id).await {
            return Ok(ProxyPeerHandle {
                peer_id: peer_id.to_string(),
                loopback_endpoint: existing,
            });
        }

        let loopback_socket = Arc::new(
            UdpSocket::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
                .await
                .with_context(|| format!("bind loopback udp proxy socket for peer {peer_id}"))?,
        );
        let loopback_addr = loopback_socket
            .local_addr()
            .context("read loopback udp proxy address")?;
        let outbound_task = tokio::spawn(run_peer_outbound_loop(
            peer_id.to_string(),
            self.public_socket.clone(),
            self.state.clone(),
            loopback_socket.clone(),
        ));

        let mut state = self.state.lock().await;
        let peer_state = ProxyPeerState {
            loopback_socket,
            loopback_addr,
            outbound_task,
            remote_endpoint: None,
        };
        state.peers.insert(peer_id.to_string(), peer_state);

        Ok(ProxyPeerHandle {
            peer_id: peer_id.to_string(),
            loopback_endpoint: socketaddr_to_endpoint(loopback_addr),
        })
    }

    pub async fn local_endpoint(&self, peer_id: &str) -> Option<Endpoint> {
        let state = self.state.lock().await;
        state
            .peers
            .get(peer_id)
            .map(|peer| socketaddr_to_endpoint(peer.loopback_addr))
    }

    pub async fn remove_peer(&self, peer_id: &str) {
        let mut state = self.state.lock().await;
        if let Some(peer) = state.peers.remove(peer_id) {
            peer.outbound_task.abort();
        }
    }

    pub async fn retain_only(&self, visible_peer_ids: &BTreeSet<String>) {
        let to_remove = {
            let state = self.state.lock().await;
            state
                .peers
                .keys()
                .filter(|peer_id| !visible_peer_ids.contains(*peer_id))
                .cloned()
                .collect::<Vec<_>>()
        };
        for peer_id in to_remove {
            self.remove_peer(&peer_id).await;
        }
    }

    pub async fn set_peer_remote_endpoint(
        &self,
        peer_id: &str,
        remote_endpoint: Option<Endpoint>,
    ) -> Result<()> {
        let mut state = self.state.lock().await;
        let Some(peer) = state.peers.get_mut(peer_id) else {
            anyhow::bail!("peer {peer_id} is not registered in public udp proxy");
        };
        peer.remote_endpoint = remote_endpoint;
        Ok(())
    }

    async fn ensure_started(&self) {
        let mut public_task = self.public_task.lock().await;
        if public_task.is_none() {
            *public_task = Some(tokio::spawn(run_public_recv_loop(
                self.public_socket.clone(),
                self.state.clone(),
            )));
        }
    }
}

impl Drop for PublicUdpProxy {
    fn drop(&mut self) {
        if let Some(public_task) = self.public_task.get_mut().take() {
            public_task.abort();
        }
    }
}

async fn run_public_recv_loop(public_socket: Arc<UdpSocket>, state: Arc<Mutex<ProxyState>>) {
    let mut buffer = [0u8; 65_535];
    loop {
        let (len, remote_addr) = match public_socket.recv_from(&mut buffer).await {
            Ok(value) => value,
            Err(err) => {
                warn!("public udp proxy receive failed: {err}");
                break;
            }
        };

        let Some(inbound_match) = match_remote_peer(&state, remote_addr).await else {
            warn!(remote_addr = %remote_addr, "dropping public udp packet with no matching peer");
            continue;
        };

        let kernel_wg_addr = {
            let state = state.lock().await;
            state.kernel_wg_addr
        };

        if let Some(observed_endpoint) = inbound_match.observed {
            let observation_tx = {
                let state = state.lock().await;
                state.observation_tx.clone()
            };
            if observation_tx.send(observed_endpoint).await.is_err() {
                warn!("public udp proxy observation channel closed");
            }
        }

        for loopback_socket in inbound_match.recipients {
            if let Err(err) = loopback_socket
                .send_to(&buffer[..len], kernel_wg_addr)
                .await
            {
                warn!(remote_addr = %remote_addr, "forward inbound public udp packet failed: {err}");
            }
        }
    }
}

async fn run_peer_outbound_loop(
    peer_id: String,
    public_socket: Arc<UdpSocket>,
    state: Arc<Mutex<ProxyState>>,
    loopback_socket: Arc<UdpSocket>,
) {
    let mut buffer = [0u8; 65_535];
    loop {
        let (len, _kernel_addr) = match loopback_socket.recv_from(&mut buffer).await {
            Ok(value) => value,
            Err(err) => {
                warn!(peer_id = %peer_id, "loopback udp proxy receive failed: {err}");
                break;
            }
        };

        let remote_endpoint = {
            let state = state.lock().await;
            state
                .peers
                .get(&peer_id)
                .and_then(|peer| peer.remote_endpoint.clone())
        };

        let Some(remote_endpoint) = remote_endpoint else {
            continue;
        };

        if let Err(err) = public_socket
            .send_to(&buffer[..len], remote_endpoint.render())
            .await
        {
            warn!(
                peer_id = %peer_id,
                endpoint = %remote_endpoint.render(),
                "forward outbound proxy packet failed: {err}"
            );
        }
    }
}

async fn match_remote_peer(
    state: &Arc<Mutex<ProxyState>>,
    remote_addr: SocketAddr,
) -> Option<InboundPeerMatch> {
    let remote_ip = remote_addr.ip();
    let remote_port = remote_addr.port();
    let locked = state.lock().await;

    let exact = locked
        .peers
        .iter()
        .find(|(_, peer)| {
            peer.remote_endpoint
                .as_ref()
                .map(|endpoint| {
                    endpoint.host == remote_ip.to_string() && endpoint.port == remote_port
                })
                .unwrap_or(false)
        })
        .map(|(peer_id, peer)| InboundPeerMatch {
            recipients: vec![peer.loopback_socket.clone()],
            observed: Some(ObservedRemoteEndpoint {
                peer_id: peer_id.clone(),
                endpoint: Endpoint {
                    host: remote_ip.to_string(),
                    port: remote_port,
                },
            })
            .filter(|observed| {
                peer.remote_endpoint
                    .as_ref()
                    .map(|endpoint| endpoint != &observed.endpoint)
                    .unwrap_or(false)
            }),
        });
    if exact.is_some() {
        return exact;
    }

    let host_matches = locked
        .peers
        .iter()
        .filter_map(|(peer_id, peer)| {
            let endpoint = peer.remote_endpoint.as_ref()?;
            if parse_endpoint_ip(endpoint) == Some(remote_ip) {
                Some((
                    peer_id.clone(),
                    peer.loopback_socket.clone(),
                    endpoint.clone(),
                ))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    drop(locked);

    if host_matches.len() != 1 {
        if let Some(port_match) = match_remote_peer_by_port(state, remote_ip, remote_port).await {
            return Some(port_match);
        }
        if let Some(single_peer_match) =
            match_remote_peer_by_single_configured_peer(state, remote_ip, remote_port).await
        {
            return Some(single_peer_match);
        }
        return match_remote_peer_by_fanout(state).await;
    }

    let (peer_id, loopback_socket, configured_endpoint) = host_matches.into_iter().next()?;
    Some(InboundPeerMatch {
        recipients: vec![loopback_socket],
        observed: observation_for_remote(&peer_id, &configured_endpoint, remote_ip, remote_port),
    })
}

async fn match_remote_peer_by_port(
    state: &Arc<Mutex<ProxyState>>,
    remote_ip: IpAddr,
    remote_port: u16,
) -> Option<InboundPeerMatch> {
    let state = state.lock().await;
    let port_matches = state
        .peers
        .iter()
        .filter_map(|(peer_id, peer)| {
            let endpoint = peer.remote_endpoint.as_ref()?;
            if endpoint.port == remote_port {
                Some((
                    peer_id.clone(),
                    peer.loopback_socket.clone(),
                    endpoint.clone(),
                ))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    if port_matches.len() != 1 {
        return None;
    }

    let (peer_id, loopback_socket, configured_endpoint) = port_matches.into_iter().next()?;
    Some(InboundPeerMatch {
        recipients: vec![loopback_socket],
        observed: observation_for_remote(&peer_id, &configured_endpoint, remote_ip, remote_port),
    })
}

async fn match_remote_peer_by_single_configured_peer(
    state: &Arc<Mutex<ProxyState>>,
    remote_ip: IpAddr,
    remote_port: u16,
) -> Option<InboundPeerMatch> {
    let state = state.lock().await;
    let configured_peers = state
        .peers
        .iter()
        .filter_map(|(peer_id, peer)| {
            let endpoint = peer.remote_endpoint.as_ref()?;
            Some((
                peer_id.clone(),
                peer.loopback_socket.clone(),
                endpoint.clone(),
            ))
        })
        .collect::<Vec<_>>();

    if configured_peers.len() != 1 {
        return None;
    }

    let (peer_id, loopback_socket, configured_endpoint) = configured_peers.into_iter().next()?;
    Some(InboundPeerMatch {
        recipients: vec![loopback_socket],
        observed: observation_for_remote(&peer_id, &configured_endpoint, remote_ip, remote_port),
    })
}

fn observation_for_remote(
    peer_id: &str,
    configured_endpoint: &Endpoint,
    remote_ip: IpAddr,
    remote_port: u16,
) -> Option<ObservedRemoteEndpoint> {
    let remote_host = remote_ip.to_string();
    if configured_endpoint.host == remote_host && configured_endpoint.port == remote_port {
        None
    } else {
        Some(ObservedRemoteEndpoint {
            peer_id: peer_id.to_string(),
            endpoint: Endpoint {
                host: remote_host,
                port: remote_port,
            },
        })
    }
}

async fn match_remote_peer_by_fanout(state: &Arc<Mutex<ProxyState>>) -> Option<InboundPeerMatch> {
    let state = state.lock().await;
    let recipients = state
        .peers
        .values()
        .filter(|peer| peer.remote_endpoint.is_some())
        .map(|peer| peer.loopback_socket.clone())
        .collect::<Vec<_>>();

    if recipients.is_empty() {
        return None;
    }

    Some(InboundPeerMatch {
        recipients,
        observed: None,
    })
}

fn parse_endpoint_ip(endpoint: &Endpoint) -> Option<IpAddr> {
    endpoint.host.parse().ok()
}

fn socketaddr_to_endpoint(addr: SocketAddr) -> Endpoint {
    Endpoint {
        host: addr.ip().to_string(),
        port: addr.port(),
    }
}

#[cfg(test)]
mod tests {
    use super::PublicUdpProxy;
    use tokio::time::{timeout, Duration};
    use wg_manager::Endpoint;

    #[tokio::test]
    async fn forwards_loopback_packets_to_public_remote_endpoint() {
        let mut proxy = PublicUdpProxy::bind(0).await.expect("bind proxy");
        let peer = proxy.ensure_peer("peer-a").await.expect("ensure peer");
        proxy
            .set_peer_remote_endpoint(
                "peer-a",
                Some(Endpoint {
                    host: "127.0.0.1".to_string(),
                    port: 0,
                }),
            )
            .await
            .expect("set placeholder endpoint");

        let remote = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind remote udp socket");
        proxy
            .set_peer_remote_endpoint(
                "peer-a",
                Some(Endpoint {
                    host: "127.0.0.1".to_string(),
                    port: remote.local_addr().expect("read remote addr").port(),
                }),
            )
            .await
            .expect("set real remote endpoint");

        let kernel = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind kernel sender");
        kernel
            .send_to(b"hello", peer.loopback_endpoint.render())
            .await
            .expect("send loopback packet");

        let mut buf = [0u8; 32];
        let (len, src) = timeout(Duration::from_secs(2), remote.recv_from(&mut buf))
            .await
            .expect("wait for remote packet")
            .expect("receive remote packet");
        assert_eq!(&buf[..len], b"hello");
        assert_eq!(
            src.port(),
            proxy.public_addr().expect("read public addr").port()
        );

        let _ = proxy.take_observation_rx();
    }

    #[tokio::test]
    async fn forwards_public_packets_to_kernel_and_reports_host_only_observation() {
        let mut proxy = PublicUdpProxy::bind(0).await.expect("bind proxy");
        let mut observation_rx = proxy.take_observation_rx().expect("take observation rx");
        let peer = proxy.ensure_peer("peer-a").await.expect("ensure peer");
        proxy
            .set_peer_remote_endpoint(
                "peer-a",
                Some(Endpoint {
                    host: "127.0.0.1".to_string(),
                    port: 40000,
                }),
            )
            .await
            .expect("set peer remote endpoint");

        let kernel =
            tokio::net::UdpSocket::bind(format!("127.0.0.1:{}", proxy.kernel_listen_port()))
                .await
                .expect("bind fake kernel listener");

        let remote = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind remote sender");
        remote
            .send_to(
                b"world",
                proxy.public_addr().expect("read public addr").to_string(),
            )
            .await
            .expect("send public packet");

        let mut buf = [0u8; 32];
        let (len, src) = timeout(Duration::from_secs(2), kernel.recv_from(&mut buf))
            .await
            .expect("wait for kernel packet")
            .expect("receive kernel packet");
        assert_eq!(&buf[..len], b"world");
        assert_eq!(src.port(), peer.loopback_endpoint.port);

        let observed = timeout(Duration::from_secs(2), observation_rx.recv())
            .await
            .expect("wait for observed endpoint")
            .expect("receive observed endpoint");
        assert_eq!(observed.peer_id, "peer-a");
        assert_eq!(
            observed.endpoint.port,
            remote.local_addr().expect("read remote sender addr").port()
        );
    }

    #[tokio::test]
    async fn forwards_public_packets_to_kernel_for_single_configured_peer_with_translated_source() {
        let mut proxy = PublicUdpProxy::bind(0).await.expect("bind proxy");
        let mut observation_rx = proxy.take_observation_rx().expect("take observation rx");
        let peer = proxy.ensure_peer("peer-a").await.expect("ensure peer");
        proxy
            .set_peer_remote_endpoint(
                "peer-a",
                Some(Endpoint {
                    host: "192.0.2.10".to_string(),
                    port: 51820,
                }),
            )
            .await
            .expect("set peer remote endpoint");

        let kernel =
            tokio::net::UdpSocket::bind(format!("127.0.0.1:{}", proxy.kernel_listen_port()))
                .await
                .expect("bind fake kernel listener");

        let remote = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind remote sender");
        remote
            .send_to(
                b"translated",
                proxy.public_addr().expect("read public addr").to_string(),
            )
            .await
            .expect("send public packet");

        let mut buf = [0u8; 32];
        let (len, src) = timeout(Duration::from_secs(2), kernel.recv_from(&mut buf))
            .await
            .expect("wait for kernel packet")
            .expect("receive kernel packet");
        assert_eq!(&buf[..len], b"translated");
        assert_eq!(src.port(), peer.loopback_endpoint.port);

        let observed = timeout(Duration::from_secs(2), observation_rx.recv())
            .await
            .expect("wait for observed endpoint")
            .expect("receive observed endpoint");
        assert_eq!(observed.peer_id, "peer-a");
        assert_eq!(observed.endpoint.host, "127.0.0.1");
        assert_eq!(
            observed.endpoint.port,
            remote.local_addr().expect("read remote sender addr").port()
        );
    }

    #[tokio::test]
    async fn fans_out_ambiguous_public_packets_to_all_configured_peers() {
        let mut proxy = PublicUdpProxy::bind(0).await.expect("bind proxy");
        let mut observation_rx = proxy.take_observation_rx().expect("take observation rx");
        let peer_a = proxy.ensure_peer("peer-a").await.expect("ensure peer-a");
        let peer_b = proxy.ensure_peer("peer-b").await.expect("ensure peer-b");
        proxy
            .set_peer_remote_endpoint(
                "peer-a",
                Some(Endpoint {
                    host: "192.0.2.10".to_string(),
                    port: 51820,
                }),
            )
            .await
            .expect("set peer-a remote endpoint");
        proxy
            .set_peer_remote_endpoint(
                "peer-b",
                Some(Endpoint {
                    host: "198.51.100.20".to_string(),
                    port: 51821,
                }),
            )
            .await
            .expect("set peer-b remote endpoint");

        let kernel =
            tokio::net::UdpSocket::bind(format!("127.0.0.1:{}", proxy.kernel_listen_port()))
                .await
                .expect("bind fake kernel listener");

        let remote = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind remote sender");
        remote
            .send_to(
                b"fanout",
                proxy.public_addr().expect("read public addr").to_string(),
            )
            .await
            .expect("send public packet");

        let mut buf = [0u8; 32];
        let (len_a, src_a) = timeout(Duration::from_secs(2), kernel.recv_from(&mut buf))
            .await
            .expect("wait for first kernel packet")
            .expect("receive first kernel packet");
        assert_eq!(&buf[..len_a], b"fanout");
        let (len_b, src_b) = timeout(Duration::from_secs(2), kernel.recv_from(&mut buf))
            .await
            .expect("wait for second kernel packet")
            .expect("receive second kernel packet");
        assert_eq!(&buf[..len_b], b"fanout");

        let received_ports = [src_a.port(), src_b.port()]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        let expected_ports = [peer_a.loopback_endpoint.port, peer_b.loopback_endpoint.port]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(received_ports, expected_ports);

        assert!(timeout(Duration::from_millis(200), observation_rx.recv())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn allocates_distinct_loopback_endpoints_for_multiple_peers() {
        let mut proxy = PublicUdpProxy::bind(0).await.expect("bind proxy");
        let peer_a = proxy.ensure_peer("peer-a").await.expect("ensure peer-a");
        let peer_b = proxy.ensure_peer("peer-b").await.expect("ensure peer-b");

        assert_ne!(peer_a.loopback_endpoint, peer_b.loopback_endpoint);
        assert_ne!(peer_a.loopback_endpoint.port, peer_b.loopback_endpoint.port);

        let _ = proxy.take_observation_rx();
    }
}
