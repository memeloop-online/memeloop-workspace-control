use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use futures::StreamExt;
use sha2::{Digest, Sha256};
use url::Url;

use super::MAX_PACKAGE_BYTES;

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) async fn download_https(
    url: &str,
    expected_sha256: &str,
) -> Result<Vec<u8>, &'static str> {
    download(url, expected_sha256, false).await
}

pub(crate) fn sanitized_source_ref(value: &str) -> Result<String, &'static str> {
    let mut url = Url::parse(value).map_err(|_| "plugin_download_rejected")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|port| port != 443)
    {
        return Err("plugin_download_rejected");
    }
    url.set_query(None);
    url.set_fragment(None);
    url.set_port(None)
        .map_err(|()| "plugin_download_rejected")?;
    Ok(url.into())
}

pub(crate) async fn download_github_release(
    repository: &str,
    tag: &str,
    asset: &str,
    expected_sha256: &str,
) -> Result<Vec<u8>, &'static str> {
    if repository.split('/').count() != 2
        || !safe_segment(tag)
        || !safe_segment(asset)
        || !repository.split('/').all(safe_segment)
    {
        return Err("plugin_download_rejected");
    }
    let url = format!("https://github.com/{repository}/releases/download/{tag}/{asset}");
    download(&url, expected_sha256, true).await
}

async fn download(
    url: &str,
    expected_sha256: &str,
    github_redirects: bool,
) -> Result<Vec<u8>, &'static str> {
    if expected_sha256.len() != 64 || !expected_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("plugin_download_rejected");
    }
    let operation = async {
        let mut current = Url::parse(url).map_err(|_| "plugin_download_rejected")?;
        for redirect in 0..=3 {
            let (host, addresses) =
                validate_and_resolve(&current, github_redirects, redirect).await?;
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(5))
                .resolve_to_addrs(&host, &addresses)
                .build()
                .map_err(|_| "plugin_download_failed")?;
            let response = client
                .get(current.clone())
                .send()
                .await
                .map_err(|_| "plugin_download_failed")?;
            if response.status().is_redirection() {
                if !github_redirects || redirect == 3 {
                    return Err("plugin_download_rejected");
                }
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or("plugin_download_rejected")?;
                current = current
                    .join(location)
                    .map_err(|_| "plugin_download_rejected")?;
                continue;
            }
            if !response.status().is_success() {
                return Err("plugin_download_failed");
            }
            if response
                .content_length()
                .is_some_and(|length| length > MAX_PACKAGE_BYTES as u64)
            {
                return Err("plugin_download_rejected");
            }
            let mut body = Vec::new();
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|_| "plugin_download_failed")?;
                if body.len().saturating_add(chunk.len()) > MAX_PACKAGE_BYTES {
                    return Err("plugin_download_rejected");
                }
                body.extend_from_slice(&chunk);
            }
            let actual = format!("{:x}", Sha256::digest(&body));
            if !actual.eq_ignore_ascii_case(expected_sha256) {
                return Err("plugin_digest_mismatch");
            }
            return Ok(body);
        }
        Err("plugin_download_rejected")
    };
    tokio::time::timeout(DOWNLOAD_TIMEOUT, operation)
        .await
        .map_err(|_| "plugin_download_failed")?
}

async fn validate_and_resolve(
    url: &Url,
    github: bool,
    redirect: usize,
) -> Result<(String, Vec<SocketAddr>), &'static str> {
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.port().is_some_and(|port| port != 443)
    {
        return Err("plugin_download_rejected");
    }
    let host = url
        .host_str()
        .ok_or("plugin_download_rejected")?
        .to_ascii_lowercase();
    if github
        && ((redirect == 0 && host != "github.com")
            || (redirect > 0 && host != "github.com" && !host.ends_with(".githubusercontent.com")))
    {
        return Err("plugin_download_rejected");
    }
    let addresses = tokio::net::lookup_host((host.as_str(), 443))
        .await
        .map_err(|_| "plugin_download_failed")?
        .collect::<Vec<_>>();
    if addresses.is_empty() || addresses.iter().any(|address| forbidden_ip(address.ip())) {
        return Err("plugin_download_rejected");
    }
    Ok((host, addresses))
}

fn forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => forbidden_v4(ip.octets()),
        IpAddr::V6(ip) => {
            ip.to_ipv4_mapped()
                .is_some_and(|mapped| forbidden_v4(mapped.octets()))
                || ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ((ip.segments()[0] & 0xfe00) == 0xfc00)
                || ((ip.segments()[0] & 0xffc0) == 0xfe80)
        }
    }
}

fn forbidden_v4([a, b, _, _]: [u8; 4]) -> bool {
    a == 0
        || a == 10
        || a == 127
        || a >= 224
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 168)
        || (a == 192 && b == 0)
        || (a == 192 && b == 88)
        || (a == 198 && matches!(b, 18 | 19 | 51))
        || (a == 203 && b == 0)
}

fn safe_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        && value != "."
        && value != ".."
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blocks_non_public_addresses() {
        assert!(forbidden_ip("127.0.0.1".parse().unwrap()));
        assert!(forbidden_ip("10.0.0.1".parse().unwrap()));
        assert!(forbidden_ip("fd00::1".parse().unwrap()));
        assert!(forbidden_ip("100.64.0.1".parse().unwrap()));
        assert!(forbidden_ip("198.18.0.1".parse().unwrap()));
        assert!(forbidden_ip("::ffff:100.64.0.1".parse().unwrap()));
        assert!(!forbidden_ip("1.1.1.1".parse().unwrap()));
    }

    #[test]
    fn source_reference_drops_url_credentials() {
        let value = sanitized_source_ref(
            "https://EXAMPLE.com/releases/plugin.mwcpkg?token=top-secret#fragment",
        )
        .unwrap();
        assert_eq!(value, "https://example.com/releases/plugin.mwcpkg");
        assert!(!value.contains("top-secret"));
        assert!(sanitized_source_ref("https://user:secret@example.com/plugin").is_err());
    }
}
