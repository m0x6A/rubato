//! The traffic URL is an outbound request a team member chose (ADR 0075, plan 189 Decision 4):
//! every connect is checked. A name is resolved here and its refused addresses dropped; an IP
//! literal, which never reaches a resolver, is checked before the request. No proxy from the
//! environment and no redirects, so the address checked is the address connected to. A server
//! that is itself local (ADR 0064) is exempt.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// Why a URL's address is refused, for the failing source's error and the log.
pub const REFUSED: &str =
    "address refused: loopback, link-local, private-use IPv6, shared or multicast (ADR 0075)";

/// Whether the server may not connect to `ip` for a workspace's traffic: loopback, link-local
/// (cloud metadata), unspecified, multicast, broadcast, IPv6 unique-local (`fc00::/7`, AWS's
/// `fd00:ec2::254`), shared address space (`100.64.0.0/10`, Alibaba's `100.100.100.200`), and the
/// IPv4-mapped forms of all of them. RFC 1918 ranges are allowed: the team's Prometheus is on one.
pub fn refused(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => refused_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => refused_v4(v4),
            None => refused_v6(v6),
        },
    }
}

fn refused_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_multicast()
        || ip.is_broadcast()
        || (a == 100 && (64..128).contains(&b))
}

fn refused_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (first & 0xffc0) == 0xfe80
        || (first & 0xfe00) == 0xfc00
}

/// The host of `url` when it is an IP literal that is refused.
pub fn literal_refused(url: &str) -> bool {
    match url::Url::parse(url)
        .ok()
        .and_then(|u| u.host().map(|h| h.to_owned()))
    {
        Some(url::Host::Ipv4(ip)) => refused(IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => refused(IpAddr::V6(ip)),
        _ => false,
    }
}

/// A resolver that keeps only addresses the guard allows; a name with none left fails.
pub struct GuardedResolver;

impl reqwest::dns::Resolve for GuardedResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let all: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            let kept: Vec<SocketAddr> = all.into_iter().filter(|a| !refused(a.ip())).collect();
            if kept.is_empty() {
                return Err(REFUSED.into());
            }
            Ok(Box::new(kept.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_refused_ranges() {
        let refused_cases = [
            "127.0.0.1",
            "127.9.9.9",
            "0.0.0.0",
            "169.254.169.254",
            "224.0.0.1",
            "255.255.255.255",
            "100.64.0.1",
            "100.100.100.200",
            "::1",
            "::",
            "fe80::1",
            "fd00:ec2::254",
            "fc00::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
        ];
        let allowed = [
            "10.0.0.5",
            "172.16.3.4",
            "192.168.1.20",
            "100.63.255.255",
            "100.128.0.1",
            "8.8.8.8",
            "2001:db8::1",
            "::ffff:192.168.1.20",
        ];
        for ip in refused_cases {
            assert!(refused(ip.parse().unwrap()), "{ip} is refused");
        }
        for ip in allowed {
            assert!(!refused(ip.parse().unwrap()), "{ip} is allowed");
        }
        assert!(literal_refused("http://[::ffff:127.0.0.1]:9090"));
        assert!(literal_refused("http://169.254.169.254/latest"));
        assert!(
            !literal_refused("http://prom.lan:9090"),
            "a name goes to the resolver"
        );
        assert!(!literal_refused("http://192.168.1.20:9090"));
        println!(
            "checked {} refused and {} allowed addresses",
            refused_cases.len(),
            allowed.len()
        );
    }
}
