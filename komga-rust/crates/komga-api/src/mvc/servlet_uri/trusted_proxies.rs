//! Trusted reverse-proxy IP/CIDR allowlist for honoring `Forwarded` and `X-Forwarded-*` headers.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;

/// A network prefix matched against a TCP peer IP address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cidr {
    V4 { network: u32, mask: u32 },
    V6 { network: u128, mask: u128 },
}

impl Cidr {
    fn v4(addr: Ipv4Addr, prefix: u8) -> Self {
        let mask = if prefix == 0 {
            0
        } else {
            !0_u32 << (32 - prefix)
        };
        Self::V4 {
            network: u32::from(addr) & mask,
            mask,
        }
    }

    fn v6(addr: Ipv6Addr, prefix: u8) -> Self {
        let mask = if prefix == 0 {
            0
        } else {
            !0_u128 << (128 - prefix)
        };
        Self::V6 {
            network: u128::from(addr) & mask,
            mask,
        }
    }

    fn contains(self, ip: IpAddr) -> bool {
        match (self, normalize_ip(ip)) {
            (Self::V4 { network, mask }, IpAddr::V4(v4)) => (u32::from(v4) & mask) == network,
            (Self::V6 { network, mask }, IpAddr::V6(v6)) => (u128::from(v6) & mask) == network,
            _ => false,
        }
    }

    fn parse(token: &str) -> Result<Self, String> {
        match token.split_once('/') {
            Some((addr_str, prefix_str)) => {
                let ip: IpAddr = addr_str
                    .parse()
                    .map_err(|_| format!("invalid IP address {addr_str:?}"))?;
                let prefix: u8 = prefix_str
                    .parse()
                    .map_err(|_| format!("invalid prefix length {prefix_str:?}"))?;
                match normalize_ip(ip) {
                    IpAddr::V4(v4) if prefix <= 32 => Ok(Self::v4(v4, prefix)),
                    IpAddr::V6(v6) if prefix <= 128 => Ok(Self::v6(v6, prefix)),
                    _ => Err(format!("prefix length {prefix} out of range for {ip}")),
                }
            }
            None => {
                let ip: IpAddr = token
                    .parse()
                    .map_err(|_| format!("invalid IP address {token:?}"))?;
                Ok(match normalize_ip(ip) {
                    IpAddr::V4(v4) => Self::v4(v4, 32),
                    IpAddr::V6(v6) => Self::v6(v6, 128),
                })
            }
        }
    }
}

/// Normalizes IPv4-mapped IPv6 addresses (`::ffff:127.0.0.1`) to their IPv4 form so dual-stack
/// sockets match IPv4 allowlist rules.
fn normalize_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4),
        IpAddr::V4(v4) => IpAddr::V4(v4),
    }
}

/// Controls which TCP peer addresses are trusted to supply `Forwarded` / `X-Forwarded-*` headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedProxies {
    cidrs: Arc<[Cidr]>,
}

impl Default for TrustedProxies {
    /// Trusts loopback and RFC 1918 / RFC 4193 private networks by default.
    fn default() -> Self {
        Self::private_networks()
    }
}

impl TrustedProxies {
    /// Loopback (`127.0.0.0/8`, `::1/128`) and private networks (`10.0.0.0/8`, `172.16.0.0/12`,
    /// `192.168.0.0/16`, `fc00::/7`).
    #[must_use]
    pub fn private_networks() -> Self {
        Self {
            cidrs: Arc::from([
                Cidr::v4(Ipv4Addr::new(127, 0, 0, 0), 8),
                Cidr::v4(Ipv4Addr::new(10, 0, 0, 0), 8),
                Cidr::v4(Ipv4Addr::new(172, 16, 0, 0), 12),
                Cidr::v4(Ipv4Addr::new(192, 168, 0, 0), 16),
                Cidr::v6(Ipv6Addr::LOCALHOST, 128),
                Cidr::v6(Ipv6Addr::new(0xfc00, 0, 0, 0, 0, 0, 0, 0), 7),
            ]),
        }
    }

    /// Trusts no peer addresses; forwarded headers are always ignored.
    #[must_use]
    pub fn none() -> Self {
        Self {
            cidrs: Arc::from([]),
        }
    }

    /// Trusts all peer addresses (`0.0.0.0/0` and `::/0`).
    #[must_use]
    pub fn all() -> Self {
        Self {
            cidrs: Arc::from([
                Cidr::v4(Ipv4Addr::UNSPECIFIED, 0),
                Cidr::v6(Ipv6Addr::UNSPECIFIED, 0),
            ]),
        }
    }

    /// Parses a comma-separated list of IP addresses / CIDRs, or the keywords `none`, `private`,
    /// or `all`.
    ///
    /// # Errors
    /// Returns an error message when any entry is not a valid IP address, CIDR, or keyword.
    pub fn parse(value: &str) -> Result<Self, String> {
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
            return Ok(Self::none());
        }
        if trimmed.eq_ignore_ascii_case("private") {
            return Ok(Self::private_networks());
        }
        if trimmed.eq_ignore_ascii_case("all") {
            return Ok(Self::all());
        }
        let mut cidrs = Vec::new();
        for token in trimmed.split(',') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            cidrs.push(Cidr::parse(token)?);
        }
        Ok(Self {
            cidrs: Arc::from(cidrs),
        })
    }

    /// Returns whether `peer` is allowed to set forwarded headers.
    #[must_use]
    pub fn is_trusted(&self, peer: Option<IpAddr>) -> bool {
        peer.is_some_and(|ip| self.cidrs.iter().any(|cidr| cidr.contains(ip)))
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use super::TrustedProxies;

    #[test]
    fn default_trusts_loopback_and_private_networks_only() {
        let proxies = TrustedProxies::default();
        for trusted in [
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5)),
            IpAddr::V4(Ipv4Addr::new(172, 16, 1, 1)),
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
            IpAddr::V6(Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 1)),
            // IPv4-mapped IPv6 loopback on dual-stack sockets.
            IpAddr::V6(Ipv4Addr::LOCALHOST.to_ipv6_mapped()),
        ] {
            assert!(proxies.is_trusted(Some(trusted)), "{trusted}");
        }
        for untrusted in [
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10)),
            IpAddr::V4(Ipv4Addr::new(172, 32, 0, 1)),
            IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
        ] {
            assert!(!proxies.is_trusted(Some(untrusted)), "{untrusted}");
        }
        assert!(!proxies.is_trusted(None));
    }

    #[test]
    fn parses_keywords_and_cidr_lists() {
        let custom = TrustedProxies::parse("203.0.113.0/24, 198.51.100.7").expect("Valid CIDRs");
        assert!(custom.is_trusted(Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 42)))));
        assert!(custom.is_trusted(Some(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7)))));
        assert!(!custom.is_trusted(Some(IpAddr::V4(Ipv4Addr::LOCALHOST))));

        assert!(
            !TrustedProxies::parse("none")
                .expect("none")
                .is_trusted(Some(IpAddr::V4(Ipv4Addr::LOCALHOST)))
        );
        assert!(
            TrustedProxies::parse("all")
                .expect("all")
                .is_trusted(Some(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))))
        );
        assert!(TrustedProxies::parse("10.0.0.0/33").is_err());
        assert!(TrustedProxies::parse("not-an-ip").is_err());
    }
}
