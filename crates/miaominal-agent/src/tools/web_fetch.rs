use crate::channel::{AgentExecChannel, ToolOutput};
use crate::error::{AgentError, AgentResult};
use reqwest::{StatusCode, Url, header::LOCATION, redirect::Policy};
use serde::Deserialize;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use std::time::Duration;

const MAX_REDIRECTS: usize = 10;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Deserialize)]
pub struct WebFetchArgs {
    pub url: String,
    pub max_bytes: Option<usize>,
}

pub async fn web_fetch(
    channel: &AgentExecChannel,
    args: WebFetchArgs,
    approved: bool,
) -> AgentResult<ToolOutput> {
    let configured_max = channel.web_fetch_config().max_bytes;
    // Tool input may lower the configured limit, but must never be able to raise it.
    let max_bytes = args.max_bytes.unwrap_or(configured_max).min(configured_max);
    let original_url = args.url;
    let mut url = parse_http_url(&original_url, approved)?;

    let mut response = None;
    for redirect_count in 0..=MAX_REDIRECTS {
        let client = client_for_url(&url, approved).await?;
        let current_response = client
            .get(url.clone())
            .send()
            .await
            .map_err(anyhow::Error::from)?;

        if is_followed_redirect(current_response.status())
            && let Some(location) = current_response.headers().get(LOCATION)
        {
            if redirect_count == MAX_REDIRECTS {
                return Err(AgentError::InvalidArguments(format!(
                    "web_fetch URL exceeded {MAX_REDIRECTS} redirects"
                )));
            }

            let location = location.to_str().map_err(|_| {
                AgentError::InvalidArguments(
                    "web_fetch redirect contains a non-UTF-8 location".into(),
                )
            })?;
            let redirected = url.join(location).map_err(|error| {
                AgentError::InvalidArguments(format!("web_fetch redirect URL is invalid: {error}"))
            })?;
            // Validation (including DNS resolution) happens again before the next request.
            validate_http_url(&redirected, approved)?;
            url = redirected;
            continue;
        }

        response = Some(current_response);
        break;
    }

    let response = response.ok_or_else(|| {
        AgentError::InvalidArguments("web_fetch could not obtain a final response".into())
    })?;
    let (bytes, truncated) = read_body_limited(response, max_bytes).await?;
    let content = utf8_lossy_limited(&bytes, max_bytes);

    Ok(ToolOutput::WebFetch {
        url: original_url,
        content,
        truncated,
    })
}

fn parse_http_url(raw: &str, approved: bool) -> AgentResult<Url> {
    let url = Url::parse(raw).map_err(|error| {
        AgentError::InvalidArguments(format!("web_fetch URL is invalid: {error}"))
    })?;
    validate_http_url(&url, approved)?;
    Ok(url)
}

fn validate_http_url(url: &Url, approved: bool) -> AgentResult<()> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AgentError::InvalidArguments(
            "web_fetch only permits http and https URLs".into(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AgentError::InvalidArguments(
            "web_fetch URLs must not contain credentials".into(),
        ));
    }

    let host = url
        .host_str()
        .ok_or_else(|| AgentError::InvalidArguments("web_fetch URL must contain a host".into()))?;
    let host = host_without_ipv6_brackets(host);
    if let Ok(ip) = host.parse::<IpAddr>() {
        enforce_address_policy(std::iter::once(ip), approved)?;
    }
    Ok(())
}

async fn client_for_url(url: &Url, approved: bool) -> AgentResult<reqwest::Client> {
    validate_http_url(url, approved)?;
    let host = url
        .host_str()
        .ok_or_else(|| AgentError::InvalidArguments("web_fetch URL must contain a host".into()))?;
    let host = host_without_ipv6_brackets(host);

    let mut builder = reqwest::Client::builder()
        .redirect(Policy::none())
        .no_proxy()
        .gzip(true)
        .no_brotli()
        .no_zstd()
        .no_deflate()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT);

    if host.parse::<IpAddr>().is_err() {
        let port = url.port_or_known_default().ok_or_else(|| {
            AgentError::InvalidArguments("web_fetch URL has no usable port".into())
        })?;
        let domain = host.to_owned();
        let lookup_domain = domain.clone();
        let mut addresses = tokio::task::spawn_blocking(move || {
            (lookup_domain.as_str(), port)
                .to_socket_addrs()
                .map(|iter| iter.collect::<Vec<_>>())
        })
        .await
        .map_err(anyhow::Error::from)?
        .map_err(anyhow::Error::from)?;

        addresses.sort_unstable();
        addresses.dedup();
        if addresses.is_empty() {
            return Err(AgentError::InvalidArguments(format!(
                "web_fetch host `{domain}` did not resolve to an address"
            )));
        }
        enforce_address_policy(addresses.iter().map(|address| address.ip()), approved)?;

        // Pin the validated addresses so the connection cannot perform a second DNS
        // lookup and be redirected to a private address (DNS rebinding).
        builder = builder.resolve_to_addrs(&domain, &addresses);
    }

    builder
        .build()
        .map_err(anyhow::Error::from)
        .map_err(Into::into)
}

fn host_without_ipv6_brackets(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddressPolicy {
    Public,
    NeedsApproval,
    Denied,
}

fn enforce_address_policy(
    addresses: impl IntoIterator<Item = IpAddr>,
    approved: bool,
) -> AgentResult<()> {
    let mut private_address = None;
    for ip in addresses {
        match address_policy(ip) {
            AddressPolicy::Public => {}
            AddressPolicy::NeedsApproval => {
                private_address.get_or_insert(ip);
            }
            AddressPolicy::Denied => {
                return Err(AgentError::InvalidArguments(format!(
                    "web_fetch refuses link-local or special-purpose address `{ip}`"
                )));
            }
        };
    }

    if private_address.is_some() && !approved {
        return Err(AgentError::ApprovalRequired {
            tool_name: "web_fetch".into(),
        });
    }
    Ok(())
}

fn address_policy(ip: IpAddr) -> AddressPolicy {
    match ip {
        IpAddr::V4(ip) => ipv4_policy(ip),
        IpAddr::V6(ip) => ipv6_policy(ip),
    }
}

fn ipv4_policy(ip: Ipv4Addr) -> AddressPolicy {
    if [
        (Ipv4Addr::new(10, 0, 0, 0), 8),
        (Ipv4Addr::new(127, 0, 0, 0), 8),
        (Ipv4Addr::new(172, 16, 0, 0), 12),
        (Ipv4Addr::new(192, 168, 0, 0), 16),
    ]
    .into_iter()
    .any(|(network, prefix)| ipv4_in_prefix(ip, network, prefix))
    {
        return AddressPolicy::NeedsApproval;
    }

    // Deny all non-forwardable and special-purpose ranges. This intentionally errs
    // on the conservative side for a tool that only needs to fetch public websites.
    if [
        (Ipv4Addr::new(0, 0, 0, 0), 8),
        (Ipv4Addr::new(100, 64, 0, 0), 10),
        (Ipv4Addr::new(169, 254, 0, 0), 16),
        (Ipv4Addr::new(192, 0, 0, 0), 24),
        (Ipv4Addr::new(192, 0, 2, 0), 24),
        (Ipv4Addr::new(192, 88, 99, 0), 24),
        (Ipv4Addr::new(198, 18, 0, 0), 15),
        (Ipv4Addr::new(198, 51, 100, 0), 24),
        (Ipv4Addr::new(203, 0, 113, 0), 24),
        (Ipv4Addr::new(224, 0, 0, 0), 4),
        (Ipv4Addr::new(240, 0, 0, 0), 4),
    ]
    .into_iter()
    .any(|(network, prefix)| ipv4_in_prefix(ip, network, prefix))
    {
        AddressPolicy::Denied
    } else {
        AddressPolicy::Public
    }
}

fn ipv6_policy(ip: Ipv6Addr) -> AddressPolicy {
    if ip == Ipv6Addr::LOCALHOST
        || ipv6_in_prefix(ip, "fc00::".parse().expect("valid IPv6 prefix"), 7)
    {
        return AddressPolicy::NeedsApproval;
    }
    if let Some(ipv4) = ipv4_mapped_address(ip) {
        return ipv4_policy(ipv4);
    }

    if [
        (Ipv6Addr::UNSPECIFIED, 96), // unspecified and IPv4-compatible addresses
        ("::ffff:0:0".parse().expect("valid IPv6 prefix"), 96),
        ("64:ff9b::".parse().expect("valid IPv6 prefix"), 96),
        ("64:ff9b:1::".parse().expect("valid IPv6 prefix"), 48),
        ("100::".parse().expect("valid IPv6 prefix"), 64),
        ("2001::".parse().expect("valid IPv6 prefix"), 23),
        ("2001:db8::".parse().expect("valid IPv6 prefix"), 32),
        ("2002::".parse().expect("valid IPv6 prefix"), 16),
        ("3fff::".parse().expect("valid IPv6 prefix"), 20),
        ("5f00::".parse().expect("valid IPv6 prefix"), 16),
        ("fe80::".parse().expect("valid IPv6 prefix"), 10),
        ("fec0::".parse().expect("valid IPv6 prefix"), 10),
        ("ff00::".parse().expect("valid IPv6 prefix"), 8),
    ]
    .into_iter()
    .any(|(network, prefix)| ipv6_in_prefix(ip, network, prefix))
    {
        AddressPolicy::Denied
    } else {
        AddressPolicy::Public
    }
}

fn ipv4_mapped_address(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let octets = ip.octets();
    if octets[..10] == [0; 10] && octets[10..12] == [0xff, 0xff] {
        Some(Ipv4Addr::new(
            octets[12], octets[13], octets[14], octets[15],
        ))
    } else {
        None
    }
}

fn ipv4_in_prefix(ip: Ipv4Addr, network: Ipv4Addr, prefix: u32) -> bool {
    let mask = u32::MAX.checked_shl(32 - prefix).unwrap_or(0);
    u32::from(ip) & mask == u32::from(network) & mask
}

fn ipv6_in_prefix(ip: Ipv6Addr, network: Ipv6Addr, prefix: u32) -> bool {
    let mask = u128::MAX.checked_shl(128 - prefix).unwrap_or(0);
    u128::from(ip) & mask == u128::from(network) & mask
}

fn is_followed_redirect(status: StatusCode) -> bool {
    matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
}

async fn read_body_limited(
    mut response: reqwest::Response,
    max_bytes: usize,
) -> AgentResult<(Vec<u8>, bool)> {
    let mut body = Vec::with_capacity(max_bytes.min(8 * 1024));

    while let Some(chunk) = response.chunk().await.map_err(anyhow::Error::from)? {
        if chunk.is_empty() {
            continue;
        }
        let remaining = max_bytes.saturating_sub(body.len());
        if remaining == 0 {
            return Ok((body, true));
        }
        if chunk.len() > remaining {
            body.extend_from_slice(&chunk[..remaining]);
            return Ok((body, true));
        }
        body.extend_from_slice(&chunk);
    }

    Ok((body, false))
}

fn utf8_lossy_limited(bytes: &[u8], max_bytes: usize) -> String {
    let mut content = String::from_utf8_lossy(bytes).into_owned();
    if content.len() > max_bytes {
        let mut end = max_bytes;
        while end > 0 && !content.is_char_boundary(end) {
            end -= 1;
        }
        content.truncate(end);
    }
    content
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_http_and_credential_urls() {
        assert!(parse_http_url("file:///etc/passwd", false).is_err());
        assert!(parse_http_url("http://user:password@example.com/", false).is_err());
    }

    #[test]
    fn localhost_and_private_ipv4_addresses_require_approval() {
        for address in ["127.0.0.1", "10.0.0.1", "172.16.0.1", "192.168.1.1"] {
            let ip = address.parse().unwrap();
            assert_eq!(
                address_policy(ip),
                AddressPolicy::NeedsApproval,
                "{address}"
            );
            assert!(matches!(
                enforce_address_policy([ip], false),
                Err(AgentError::ApprovalRequired { .. })
            ));
            assert!(enforce_address_policy([ip], true).is_ok());
        }

        // The URL parser canonicalizes non-standard IPv4 spellings before validation.
        assert!(matches!(
            parse_http_url("http://2130706433/", false),
            Err(AgentError::ApprovalRequired { .. })
        ));
        assert!(parse_http_url("http://2130706433/", true).is_ok());
    }

    #[test]
    fn metadata_and_special_ipv4_addresses_remain_denied() {
        for address in ["169.254.169.254", "100.64.0.1", "224.0.0.1"] {
            let ip = address.parse().unwrap();
            assert_eq!(address_policy(ip), AddressPolicy::Denied, "{address}");
            assert!(matches!(
                enforce_address_policy([ip], true),
                Err(AgentError::InvalidArguments(_))
            ));
        }
        assert_eq!(
            address_policy("8.8.8.8".parse().unwrap()),
            AddressPolicy::Public
        );
    }

    #[tokio::test]
    async fn localhost_hostname_requires_approval_after_dns_resolution() {
        let url = parse_http_url("http://localhost/", false).unwrap();
        assert!(matches!(
            client_for_url(&url, false).await,
            Err(AgentError::ApprovalRequired { .. })
        ));
        assert!(client_for_url(&url, true).await.is_ok());
    }

    #[test]
    fn localhost_and_private_ipv6_addresses_require_approval() {
        for address in ["::1", "::ffff:127.0.0.1", "fc00::1"] {
            let ip = address.parse().unwrap();
            assert_eq!(
                address_policy(ip),
                AddressPolicy::NeedsApproval,
                "{address}"
            );
            assert!(matches!(
                enforce_address_policy([ip], false),
                Err(AgentError::ApprovalRequired { .. })
            ));
            assert!(enforce_address_policy([ip], true).is_ok());
        }
        assert!(matches!(
            parse_http_url("http://[::1]/", false),
            Err(AgentError::ApprovalRequired { .. })
        ));
        assert!(parse_http_url("http://[::1]/", true).is_ok());
    }

    #[test]
    fn link_local_and_special_ipv6_addresses_remain_denied() {
        for address in ["::", "64:ff9b::127.0.0.1", "fe80::1", "ff02::1"] {
            let ip = address.parse().unwrap();
            assert_eq!(address_policy(ip), AddressPolicy::Denied, "{address}");
            assert!(matches!(
                enforce_address_policy([ip], true),
                Err(AgentError::InvalidArguments(_))
            ));
        }
        assert_eq!(
            address_policy("2606:4700:4700::1111".parse().unwrap()),
            AddressPolicy::Public
        );
    }

    #[test]
    fn redirect_targets_are_validated() {
        let base = parse_http_url("https://example.com/path", false).unwrap();
        let redirected = base
            .join("http://169.254.169.254/latest/meta-data")
            .unwrap();
        assert!(validate_http_url(&redirected, true).is_err());

        let private_redirect = base.join("http://192.168.1.1/admin").unwrap();
        assert!(matches!(
            validate_http_url(&private_redirect, false),
            Err(AgentError::ApprovalRequired { .. })
        ));
        assert!(validate_http_url(&private_redirect, true).is_ok());
    }

    #[test]
    fn denied_address_wins_over_private_address_in_mixed_dns_results() {
        assert!(matches!(
            enforce_address_policy(
                [
                    "10.0.0.1".parse().unwrap(),
                    "169.254.169.254".parse().unwrap()
                ],
                false,
            ),
            Err(AgentError::InvalidArguments(_))
        ));
    }

    #[test]
    fn lossy_utf8_output_still_obeys_the_byte_limit() {
        let content = utf8_lossy_limited(&[0xff, 0xff, b'a'], 2);
        assert!(content.len() <= 2);
    }
}
