use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use typesafe_rs::Client;
use url::{Host, Url};

/// Pin the SDK to the same addresses excluded by the collector. Disabling proxy
/// discovery prevents a proxy connection from bypassing those exclusions.
pub async fn configure(enabled: bool) -> Result<(Option<Client>, HashSet<SocketAddr>)> {
    if !enabled {
        return Ok((None, HashSet::new()));
    }
    let base = std::env::var(typesafe_rs::BASE_URL_ENV)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| typesafe_rs::DEFAULT_BASE_URL.into());
    // Validate SDK configuration before doing network work.
    let _ = Client::builder()
        .base_url(&base)
        .build()
        .context("--assess requires valid TypeSafe configuration (TYPESAFE_API_KEY)")?;
    let url = Url::parse(&base).context("invalid TypeSafe base URL")?;
    let peers = resolve(&url).await?;
    let addresses: Vec<_> = peers.iter().copied().collect();
    let mut http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none());
    if let Some(Host::Domain(host)) = url.host() {
        http = http.resolve_to_addrs(host, &addresses);
    }
    let client = Client::builder()
        .base_url(base)
        .http_client(http.build()?)
        .timeout(Duration::from_secs(10))
        .retry(typesafe_rs::RetryPolicy {
            max_retries: 1,
            ..Default::default()
        })
        .build()?;
    Ok((Some(client), peers))
}

async fn resolve(url: &Url) -> Result<HashSet<SocketAddr>> {
    let port = url
        .port_or_known_default()
        .context("TypeSafe URL has no port")?;
    let peers = match url.host().context("TypeSafe URL has no host")? {
        Host::Ipv4(ip) => HashSet::from([SocketAddr::new(IpAddr::V4(ip), port)]),
        Host::Ipv6(ip) => HashSet::from([SocketAddr::new(IpAddr::V6(ip), port)]),
        Host::Domain(host) => tokio::time::timeout(
            Duration::from_secs(10),
            tokio::net::lookup_host((host, port)),
        )
        .await
        .context("TypeSafe DNS resolution timed out")??
        .collect(),
    };
    if peers.is_empty() {
        bail!("TypeSafe hostname resolved to no addresses");
    }
    Ok(peers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn literal_endpoints_support_both_ip_families() {
        assert_eq!(
            resolve(&Url::parse("http://127.0.0.1:8080").unwrap())
                .await
                .unwrap(),
            HashSet::from(["127.0.0.1:8080".parse().unwrap()])
        );
        assert_eq!(
            resolve(&Url::parse("https://[::1]").unwrap())
                .await
                .unwrap(),
            HashSet::from(["[::1]:443".parse().unwrap()])
        );
    }

    #[tokio::test]
    async fn local_mode_needs_neither_credentials_nor_dns() {
        let (client, excluded) = configure(false).await.unwrap();
        assert!(client.is_none());
        assert!(excluded.is_empty());
    }
}
