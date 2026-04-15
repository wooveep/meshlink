use anyhow::{Context, Result};
use api_client::proto::relay_service_client::RelayServiceClient;
use api_client::proto::{ReleasePeerRelayRequest, ReservePeerRelayRequest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayReservation {
    pub relay_host: String,
    pub udp_port: u16,
    pub ttl_seconds: u32,
    pub session_id: String,
}

pub async fn reserve_peer_relay(
    relay_addr: &str,
    device_id: &str,
    public_key: &str,
    bootstrap_token: &str,
    peer_id: &str,
) -> Result<RelayReservation> {
    let mut client = RelayServiceClient::connect(api_client::normalize_endpoint(relay_addr))
        .await
        .context("connect relay service")?;

    let response = client
        .reserve_peer_relay(ReservePeerRelayRequest {
            device_id: device_id.to_string(),
            public_key: public_key.to_string(),
            bootstrap_token: bootstrap_token.to_string(),
            peer_id: peer_id.to_string(),
        })
        .await
        .context("reserve peer relay")?
        .into_inner();

    Ok(RelayReservation {
        relay_host: response.relay_host,
        udp_port: u16::try_from(response.udp_port).context("relay udp_port out of range")?,
        ttl_seconds: response.ttl_seconds,
        session_id: response.session_id,
    })
}

pub async fn release_peer_relay(
    relay_addr: &str,
    device_id: &str,
    public_key: &str,
    bootstrap_token: &str,
    peer_id: &str,
    session_id: &str,
    reason: &str,
) -> Result<()> {
    let mut client = RelayServiceClient::connect(api_client::normalize_endpoint(relay_addr))
        .await
        .context("connect relay service")?;

    client
        .release_peer_relay(ReleasePeerRelayRequest {
            device_id: device_id.to_string(),
            public_key: public_key.to_string(),
            bootstrap_token: bootstrap_token.to_string(),
            peer_id: peer_id.to_string(),
            session_id: session_id.to_string(),
            reason: reason.to_string(),
        })
        .await
        .context("release peer relay")?;

    Ok(())
}
