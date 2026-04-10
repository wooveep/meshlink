#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathPreference {
    Lan,
    Ipv6,
    PublicIpv4,
    HolepunchedIpv4,
    Relay,
}
