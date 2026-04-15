use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use tokio::net::UdpSocket;
use tokio::time::timeout;

const STUN_BINDING_REQUEST: u16 = 0x0001;
const STUN_XOR_MAPPED_ADDRESS: u16 = 0x0020;
const STUN_HEADER_LENGTH: usize = 20;
const STUN_MAGIC_COOKIE: u32 = 0x2112A442;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StunResult {
    pub address: String,
    pub port: u16,
}

pub async fn query(stun_addr: &str, wait_timeout: Duration) -> Result<StunResult> {
    query_with_bind_addr(stun_addr, wait_timeout, "0.0.0.0:0").await
}

pub async fn query_with_local_port(
    stun_addr: &str,
    wait_timeout: Duration,
    local_port: u16,
) -> Result<StunResult> {
    query_with_bind_addr(stun_addr, wait_timeout, &format!("0.0.0.0:{local_port}")).await
}

async fn query_with_bind_addr(
    stun_addr: &str,
    wait_timeout: Duration,
    bind_addr: &str,
) -> Result<StunResult> {
    let transaction_id = generate_transaction_id();
    let request = encode_binding_request(transaction_id);
    let socket = UdpSocket::bind(bind_addr)
        .await
        .with_context(|| format!("bind stun udp socket on {bind_addr}"))?;

    socket
        .send_to(&request, stun_addr)
        .await
        .with_context(|| format!("send stun binding request to {stun_addr}"))?;

    let mut buffer = [0u8; 2048];
    let (received, _) = timeout(wait_timeout, socket.recv_from(&mut buffer))
        .await
        .context("wait for stun response timed out")?
        .context("receive stun response")?;

    decode_binding_response(&buffer[..received], transaction_id)
}

pub fn encode_binding_request(transaction_id: [u8; 12]) -> Vec<u8> {
    let mut packet = vec![0u8; STUN_HEADER_LENGTH];
    packet[0..2].copy_from_slice(&STUN_BINDING_REQUEST.to_be_bytes());
    packet[2..4].copy_from_slice(&0u16.to_be_bytes());
    packet[4..8].copy_from_slice(&STUN_MAGIC_COOKIE.to_be_bytes());
    packet[8..20].copy_from_slice(&transaction_id);
    packet
}

pub fn decode_binding_response(packet: &[u8], transaction_id: [u8; 12]) -> Result<StunResult> {
    if packet.len() < STUN_HEADER_LENGTH {
        bail!("stun packet too short");
    }
    if u16::from_be_bytes([packet[0], packet[1]]) != 0x0101 {
        bail!("stun response is not a binding success");
    }
    if u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]) != STUN_MAGIC_COOKIE {
        bail!("stun response cookie mismatch");
    }
    if packet[8..20] != transaction_id {
        bail!("stun transaction id mismatch");
    }

    let body_len = u16::from_be_bytes([packet[2], packet[3]]) as usize;
    if packet.len() < STUN_HEADER_LENGTH + body_len {
        bail!("stun body is truncated");
    }

    let mut offset = STUN_HEADER_LENGTH;
    while offset + 4 <= STUN_HEADER_LENGTH + body_len {
        let attr_type = u16::from_be_bytes([packet[offset], packet[offset + 1]]);
        let attr_len = u16::from_be_bytes([packet[offset + 2], packet[offset + 3]]) as usize;
        offset += 4;
        if offset + attr_len > packet.len() {
            bail!("stun attribute is truncated");
        }

        if attr_type == STUN_XOR_MAPPED_ADDRESS {
            if attr_len < 8 {
                bail!("xor-mapped-address too short");
            }
            let family = packet[offset + 1];
            if family != 0x01 {
                bail!("only ipv4 stun results are supported");
            }

            let x_port = u16::from_be_bytes([packet[offset + 2], packet[offset + 3]]);
            let port = x_port ^ ((STUN_MAGIC_COOKIE >> 16) as u16);
            let cookie = STUN_MAGIC_COOKIE.to_be_bytes();
            let ip = [
                packet[offset + 4] ^ cookie[0],
                packet[offset + 5] ^ cookie[1],
                packet[offset + 6] ^ cookie[2],
                packet[offset + 7] ^ cookie[3],
            ];

            return Ok(StunResult {
                address: format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]),
                port,
            });
        }

        offset += attr_len;
        if attr_len % 4 != 0 {
            offset += 4 - (attr_len % 4);
        }
    }

    Err(anyhow!("stun response missing xor-mapped-address"))
}

fn generate_transaction_id() -> [u8; 12] {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let bytes = now.to_be_bytes();
    let mut transaction_id = [0u8; 12];
    transaction_id.copy_from_slice(&bytes[4..16]);
    transaction_id
}

#[cfg(test)]
mod tests {
    use super::{decode_binding_response, encode_binding_request, StunResult, STUN_MAGIC_COOKIE};

    #[test]
    fn encode_binding_request_writes_header() {
        let packet = encode_binding_request([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);

        assert_eq!(&packet[0..2], &0x0001u16.to_be_bytes());
        assert_eq!(&packet[4..8], &STUN_MAGIC_COOKIE.to_be_bytes());
        assert_eq!(&packet[8..20], &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    }

    #[test]
    fn decode_binding_response_returns_public_ipv4_mapping() {
        let transaction_id = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let cookie = STUN_MAGIC_COOKIE.to_be_bytes();
        let response = vec![
            0x01,
            0x01,
            0x00,
            0x0c,
            cookie[0],
            cookie[1],
            cookie[2],
            cookie[3],
            1,
            2,
            3,
            4,
            5,
            6,
            7,
            8,
            9,
            10,
            11,
            12,
            0x00,
            0x20,
            0x00,
            0x08,
            0x00,
            0x01,
            0xF5,
            0x23,
            198 ^ cookie[0],
            51 ^ cookie[1],
            100 ^ cookie[2],
            10 ^ cookie[3],
        ];

        let result = decode_binding_response(&response, transaction_id).expect("decode stun");
        assert_eq!(
            result,
            StunResult {
                address: "198.51.100.10".to_string(),
                port: 54321,
            }
        );
    }
}
