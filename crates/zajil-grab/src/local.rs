//! Local and private addresses, which a crawl never requests unless the
//! user started there or named the host (owner's decision on ADR-0010,
//! section 11).
//!
//! The URL parser (WHATWG URL, which `url` follows) already reads the odd
//! numeric forms of an IPv4 address as a browser does (`2130706433`,
//! `0x7f.1`, `0177.0.0.1` and `127.1` are all `127.0.0.1`), so the check
//! works on the parsed host, never on the text.

use std::net::{Ipv4Addr, Ipv6Addr};

use zajil_core::Url;

/// Whether `url`'s host is this computer or a private network: loopback,
/// private ranges, link-local, CGNAT, `0.0.0.0/8`, `::`, `::1`,
/// `fe80::/10`, `fc00::/7`, an IPv4 address inside IPv6, or `localhost`.
pub fn is_local(url: &Url) -> bool {
    // No host: not a URL a crawl requests; treat it as local.
    let Some(host) = url.host_str() else {
        return true;
    };
    // The parser writes an IPv4 host in dotted form and an IPv6 host in
    // brackets, whatever form the link used.
    if let Some(v6) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        return v6.parse().is_ok_and(is_local_v6);
    }
    if let Ok(v4) = host.parse() {
        return is_local_v4(v4);
    }
    let name = host.strip_suffix('.').unwrap_or(host).to_ascii_lowercase();
    name == "localhost" || name.ends_with(".localhost")
}

fn is_local_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    a == 0 // 0.0.0.0/8, "this network"
        || a == 10 // 10.0.0.0/8
        || a == 127 // 127.0.0.0/8, loopback
        || (a == 100 && (64..128).contains(&b)) // 100.64.0.0/10, CGNAT
        || (a == 169 && b == 254) // 169.254.0.0/16, link-local
        || (a == 172 && (16..32).contains(&b)) // 172.16.0.0/12
        || (a == 192 && b == 168) // 192.168.0.0/16
}

fn is_local_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    if ip.is_unspecified() || ip.is_loopback() {
        return true;
    }
    if first & 0xffc0 == 0xfe80 || first & 0xfe00 == 0xfc00 {
        return true; // fe80::/10 link-local, fc00::/7 unique local
    }
    // An IPv4 address inside IPv6: `::ffff:a.b.c.d` (mapped) and the old
    // `::a.b.c.d` (compatible).
    let s = ip.segments();
    if s[..5] == [0; 5] && (s[5] == 0xffff || s[5] == 0) {
        let v4 = Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8);
        return is_local_v4(v4);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(url: &str) -> bool {
        is_local(&Url::parse(url).unwrap_or_else(|e| panic!("{url}: {e}")))
    }

    /// One test per range in the owner's list, at both ends of each.
    #[test]
    fn loopback_127_0_0_0_8() {
        assert!(local("http://127.0.0.1/"));
        assert!(local("http://127.255.255.254/"));
        assert!(!local("http://126.255.255.255/"));
        assert!(!local("http://128.0.0.1/"));
    }

    #[test]
    fn private_10_0_0_0_8() {
        assert!(local("http://10.0.0.1/"));
        assert!(local("http://10.255.255.255/"));
        assert!(!local("http://11.0.0.1/"));
        assert!(!local("http://9.255.255.255/"));
    }

    #[test]
    fn private_172_16_0_0_12() {
        assert!(local("http://172.16.0.1/"));
        assert!(local("http://172.31.255.255/"));
        assert!(!local("http://172.15.255.255/"));
        assert!(!local("http://172.32.0.0/"));
    }

    #[test]
    fn private_192_168_0_0_16() {
        assert!(local("http://192.168.0.1/"));
        assert!(local("http://192.168.255.255/"));
        assert!(!local("http://192.169.0.1/"));
        assert!(!local("http://192.167.255.255/"));
    }

    #[test]
    fn link_local_169_254_0_0_16() {
        assert!(local("http://169.254.169.254/"));
        assert!(local("http://169.254.0.0/"));
        assert!(!local("http://169.253.255.255/"));
        assert!(!local("http://169.255.0.0/"));
    }

    #[test]
    fn cgnat_100_64_0_0_10() {
        assert!(local("http://100.64.0.1/"));
        assert!(local("http://100.127.255.255/"));
        assert!(!local("http://100.63.255.255/"));
        assert!(!local("http://100.128.0.0/"));
    }

    #[test]
    fn this_network_0_0_0_0() {
        assert!(local("http://0.0.0.0/"));
        assert!(local("http://0.1.2.3/"));
        assert!(local("http://0/"), "`0` is 0.0.0.0");
    }

    #[test]
    fn ipv6_loopback_and_unspecified() {
        assert!(local("http://[::1]/"));
        assert!(local("http://[0:0:0:0:0:0:0:1]:8080/"));
        assert!(local("http://[::]/"));
    }

    #[test]
    fn ipv6_link_local_fe80_10() {
        assert!(local("http://[fe80::1]/"));
        assert!(local("http://[febf:ffff::1]/"));
        assert!(!local("http://[fec0::1]/"));
        assert!(!local("http://[fe7f::1]/"));
    }

    #[test]
    fn ipv6_unique_local_fc00_7() {
        assert!(local("http://[fc00::1]/"));
        assert!(local("http://[fdff:ffff::1]/"));
        assert!(!local("http://[fe00::1]/"));
        assert!(!local("http://[fbff::1]/"));
    }

    #[test]
    fn ipv4_inside_ipv6() {
        assert!(local("http://[::ffff:127.0.0.1]/"));
        assert!(local("http://[::ffff:7f00:1]/"));
        assert!(local("http://[::ffff:192.168.1.1]/"));
        assert!(local("http://[::ffff:10.0.0.1]/"));
        assert!(local("http://[::127.0.0.1]/"), "IPv4-compatible");
        assert!(!local("http://[::ffff:8.8.8.8]/"));
    }

    /// Odd numeric forms, read as a browser reads them.
    #[test]
    fn odd_numeric_forms() {
        for url in [
            "http://2130706433/",      // 127.0.0.1 as one number
            "http://0x7f.1/",          // hex, two parts
            "http://0x7f000001/",      // hex, one part
            "http://0177.0.0.1/",      // octal
            "http://127.1/",           // two parts
            "http://127.0.1/",         // three parts
            "http://0x7F.0.0.0x1/",    // mixed
            "http://3232235777/",      // 192.168.1.1
            "http://0xa.0.0.1/",       // 10.0.0.1
            "http://012.1/",           // 10.0.0.1, octal
            "http://169.254.43518/",   // 169.254.169.254
            "http://0300.0250.1.1/",   // 192.168.1.1, octal
            "http://%31%32%37.0.0.1/", // percent-encoded 127.0.0.1
            "http://127.0.0.1./",      // trailing dot
            "http://１２７.０.０.１/", // full-width digits (#112, item 2)
        ] {
            assert!(local(url), "{url}");
        }
        assert!(!local("http://134744072/"), "8.8.8.8 as one number");
    }

    #[test]
    fn localhost_names() {
        assert!(local("http://localhost/"));
        assert!(local("http://LOCALHOST:8080/"));
        assert!(local("http://localhost./"));
        assert!(local("http://a.b.localhost/"));
        assert!(!local("http://localhost.example.com/"));
        assert!(!local("http://notlocalhost/"));
    }

    #[test]
    fn public_addresses_are_not_local() {
        for url in [
            "http://example.com/",
            "https://8.8.8.8/",
            "http://[2001:db8::1]/",
            "http://[2606:4700::1111]/",
        ] {
            assert!(!local(url), "{url}");
        }
    }
}
