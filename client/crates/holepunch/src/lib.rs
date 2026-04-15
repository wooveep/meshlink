use std::net::Ipv4Addr;

use api_client::proto::{Candidate, CandidateType, PathType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathPreference {
    Lan,
    Ipv6,
    PublicIpv4,
    HolepunchedIpv4,
    Relay,
}

pub fn sort_candidates(mut candidates: Vec<Candidate>) -> Vec<Candidate> {
    candidates.sort_by(|left, right| {
        candidate_rank(right)
            .cmp(&candidate_rank(left))
            .then_with(|| right.priority.cmp(&left.priority))
            .then_with(|| left.address.cmp(&right.address))
            .then_with(|| left.port.cmp(&right.port))
    });
    candidates
}

pub fn should_initiate(self_device_id: &str, peer_device_id: &str) -> bool {
    !self_device_id.is_empty() && !peer_device_id.is_empty() && self_device_id < peer_device_id
}

pub fn select_remote_candidate(candidates: &[Candidate]) -> Option<Candidate> {
    sort_candidates(candidates.to_vec()).into_iter().next()
}

pub fn select_remote_candidate_for_local(
    candidates: &[Candidate],
    local_candidates: &[Candidate],
) -> Option<Candidate> {
    let mut ranked = candidates.to_vec();
    ranked.sort_by(|left, right| {
        candidate_rank_for_local(right, local_candidates)
            .cmp(&candidate_rank_for_local(left, local_candidates))
            .then_with(|| right.priority.cmp(&left.priority))
            .then_with(|| left.address.cmp(&right.address))
            .then_with(|| left.port.cmp(&right.port))
    });
    ranked.into_iter().next()
}

pub fn candidate_path_type(candidate: &Candidate) -> PathType {
    match CandidateType::try_from(candidate.r#type).ok() {
        Some(CandidateType::Lan) => PathType::Lan,
        Some(CandidateType::PublicIpv4) => PathType::HolepunchedIpv4,
        Some(CandidateType::PublicIpv6) => PathType::Ipv6,
        Some(CandidateType::Relay) => PathType::Relay,
        _ => PathType::Unspecified,
    }
}

fn candidate_rank(candidate: &Candidate) -> u8 {
    match CandidateType::try_from(candidate.r#type).ok() {
        Some(CandidateType::Lan) => 3,
        Some(CandidateType::PublicIpv4) => 2,
        Some(CandidateType::PublicIpv6) => 1,
        Some(CandidateType::Relay) => 0,
        _ => 0,
    }
}

fn candidate_rank_for_local(candidate: &Candidate, local_candidates: &[Candidate]) -> u8 {
    match CandidateType::try_from(candidate.r#type).ok() {
        Some(CandidateType::Lan) if lan_candidate_reachable(candidate, local_candidates) => 3,
        Some(CandidateType::PublicIpv4) => 2,
        Some(CandidateType::Lan) => 1,
        Some(CandidateType::PublicIpv6) => 1,
        Some(CandidateType::Relay) => 0,
        _ => 0,
    }
}

fn lan_candidate_reachable(candidate: &Candidate, local_candidates: &[Candidate]) -> bool {
    let Ok(remote) = candidate.address.parse::<Ipv4Addr>() else {
        return false;
    };

    local_candidates.iter().any(|local| {
        matches!(
            CandidateType::try_from(local.r#type).ok(),
            Some(CandidateType::Lan)
        ) && local
            .address
            .parse::<Ipv4Addr>()
            .map(|local_ip| same_v4_subnet_24(local_ip, remote))
            .unwrap_or(false)
    })
}

fn same_v4_subnet_24(left: Ipv4Addr, right: Ipv4Addr) -> bool {
    let left = left.octets();
    let right = right.octets();
    left[0..3] == right[0..3]
}

#[cfg(test)]
mod tests {
    use super::{
        candidate_path_type, select_remote_candidate, select_remote_candidate_for_local,
        should_initiate, sort_candidates,
    };
    use api_client::proto::{Candidate, CandidateType, PathType};

    #[test]
    fn sort_candidates_prefers_lan_then_public_ipv4() {
        let ordered = sort_candidates(vec![
            candidate(CandidateType::PublicIpv4, "198.51.100.10", 51820, 200),
            candidate(CandidateType::Lan, "10.0.0.10", 51820, 100),
            candidate(CandidateType::PublicIpv4, "203.0.113.10", 51820, 50),
        ]);

        assert_eq!(ordered[0].address, "10.0.0.10");
        assert_eq!(ordered[1].address, "198.51.100.10");
    }

    #[test]
    fn smaller_device_id_becomes_initiator() {
        assert!(should_initiate("dev-a", "dev-b"));
        assert!(!should_initiate("dev-b", "dev-a"));
    }

    #[test]
    fn select_remote_candidate_returns_best_ranked_candidate() {
        let candidate = select_remote_candidate(&[
            candidate(CandidateType::PublicIpv4, "198.51.100.10", 51820, 200),
            candidate(CandidateType::Lan, "10.0.0.10", 51820, 100),
        ])
        .expect("best candidate");

        assert_eq!(candidate.address, "10.0.0.10");
    }

    #[test]
    fn select_remote_candidate_for_local_prefers_public_when_lan_is_not_same_subnet() {
        let candidate = select_remote_candidate_for_local(
            &[
                candidate(CandidateType::PublicIpv4, "198.51.100.10", 51820, 200),
                candidate(CandidateType::Lan, "10.10.2.10", 51820, 300),
            ],
            &[candidate(CandidateType::Lan, "10.10.1.10", 51820, 300)],
        )
        .expect("best candidate");

        assert_eq!(candidate.address, "198.51.100.10");
    }

    #[test]
    fn select_remote_candidate_for_local_keeps_lan_when_same_subnet() {
        let candidate = select_remote_candidate_for_local(
            &[
                candidate(CandidateType::PublicIpv4, "198.51.100.10", 51820, 200),
                candidate(CandidateType::Lan, "10.10.1.20", 51820, 300),
            ],
            &[candidate(CandidateType::Lan, "10.10.1.10", 51820, 300)],
        )
        .expect("best candidate");

        assert_eq!(candidate.address, "10.10.1.20");
    }

    #[test]
    fn public_ipv4_candidate_maps_to_holepunched_path() {
        let path = candidate_path_type(&candidate(
            CandidateType::PublicIpv4,
            "198.51.100.10",
            51820,
            200,
        ));

        assert_eq!(path, PathType::HolepunchedIpv4);
    }

    fn candidate(kind: CandidateType, address: &str, port: u32, priority: u32) -> Candidate {
        Candidate {
            r#type: kind as i32,
            address: address.to_string(),
            port,
            priority,
            ..Default::default()
        }
    }
}
