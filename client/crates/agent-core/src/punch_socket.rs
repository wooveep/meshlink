use std::{net::SocketAddr, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use tokio::net::UdpSocket;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PunchSocketState {
    pub requested_port: Option<u16>,
    pub local_addr: SocketAddr,
    pub port_aligned: bool,
    pub shared_receive_runtime: bool,
    pub last_public_mapping: Option<stun::StunResult>,
}

#[derive(Debug)]
pub struct PunchSocket {
    socket: Arc<UdpSocket>,
    state: PunchSocketState,
}

impl PunchSocket {
    pub async fn bind(preferred_port: Option<u16>) -> Result<Self> {
        Self::bind_with_requested_port(preferred_port, preferred_port).await
    }

    pub async fn bind_with_requested_port(
        bind_port: Option<u16>,
        requested_port: Option<u16>,
    ) -> Result<Self> {
        let requested_addr = bind_port
            .filter(|port| *port > 0)
            .map(|port| format!("0.0.0.0:{port}"))
            .unwrap_or_else(|| "0.0.0.0:0".to_string());

        let socket = match UdpSocket::bind(&requested_addr).await {
            Ok(socket) => socket,
            Err(primary_err) if bind_port.is_some() => {
                UdpSocket::bind("0.0.0.0:0").await.with_context(|| {
                    format!(
                        "bind punch socket on {} after preferred bind failed: {primary_err}",
                        requested_addr
                    )
                })?
            }
            Err(err) => {
                return Err(err).with_context(|| format!("bind punch socket on {requested_addr}"));
            }
        };

        let local_addr = socket
            .local_addr()
            .context("read punch socket local address")?;
        Ok(Self::from_bound_socket(
            Arc::new(socket),
            requested_port.filter(|port| *port > 0),
            local_addr,
            false,
        ))
    }

    pub fn from_socket(socket: Arc<UdpSocket>, requested_port: Option<u16>) -> Result<Self> {
        let local_addr = socket
            .local_addr()
            .context("read punch socket local address")?;
        Ok(Self::from_bound_socket(
            socket,
            requested_port,
            local_addr,
            true,
        ))
    }

    pub fn state(&self) -> PunchSocketState {
        self.state.clone()
    }

    pub async fn query_stun(
        &mut self,
        stun_addr: &str,
        wait_timeout: Duration,
    ) -> Result<stun::StunResult> {
        let result = stun::query_on_socket(&self.socket, stun_addr, wait_timeout).await?;
        self.state.last_public_mapping = Some(result.clone());
        Ok(result)
    }

    fn from_bound_socket(
        socket: Arc<UdpSocket>,
        requested_port: Option<u16>,
        local_addr: SocketAddr,
        shared_receive_runtime: bool,
    ) -> Self {
        Self {
            socket,
            state: PunchSocketState {
                requested_port,
                local_addr,
                port_aligned: requested_port
                    .map(|port| port == local_addr.port())
                    .unwrap_or(true),
                shared_receive_runtime,
                last_public_mapping: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PunchSocket;

    #[tokio::test]
    async fn bind_without_preferred_port_uses_ephemeral_socket() {
        let socket = PunchSocket::bind(None).await.expect("bind punch socket");
        let state = socket.state();
        assert_eq!(state.requested_port, None);
        assert_ne!(state.local_addr.port(), 0);
        assert!(state.port_aligned);
    }

    #[tokio::test]
    async fn bind_with_separate_requested_port_marks_socket_non_aligned() {
        let socket = PunchSocket::bind_with_requested_port(None, Some(51830))
            .await
            .expect("bind punch socket");
        let state = socket.state();
        assert_eq!(state.requested_port, Some(51830));
        assert_ne!(state.local_addr.port(), 0);
        assert!(!state.port_aligned);
    }
}
