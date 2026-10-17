//! A small blocking JSON client that honours `HTTPS_PROXY`.

use anyhow::{Result, anyhow};
use std::{sync::OnceLock, time::Duration};

fn agent(url: &str) -> &'static ureq::Agent {
    static PROXIED: OnceLock<ureq::Agent> = OnceLock::new();
    static DIRECT: OnceLock<ureq::Agent> = OnceLock::new();
    let build = |proxy: bool| {
        ureq::AgentBuilder::new()
            .try_proxy_from_env(proxy)
            .timeout(Duration::from_secs(180))
            .build()
    };
    if bypasses_proxy(url) {
        DIRECT.get_or_init(|| build(false))
    } else {
        PROXIED.get_or_init(|| build(true))
    }
}

/// Loopback hosts and hosts matching `NO_PROXY` are reached directly, so a
/// local model server keeps working behind a corporate or sandbox proxy.
fn bypasses_proxy(url: &str) -> bool {
    let host = url
        .split_once("://")
        .map_or(url, |(_, rest)| rest)
        .split(['/', '?'])
        .next()
        .unwrap_or_default();
    let host = host
        .rsplit_once(':')
        .filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit()))
        .map_or(host, |(host, _)| host)
        .trim_matches(['[', ']']);
    if matches!(host, "localhost" | "127.0.0.1" | "::1") {
        return true;
    }
    let no_proxy = std::env::var("NO_PROXY")
        .or_else(|_| std::env::var("no_proxy"))
        .unwrap_or_default();
    no_proxy.split(',').map(str::trim).any(|entry| {
        let entry = entry.trim_start_matches('.');
        entry == "*"
            || (!entry.is_empty() && (host == entry || host.ends_with(&format!(".{entry}"))))
    })
}

pub(crate) fn post_json(
    url: &str,
    headers: &[(&str, &str)],
    body: &serde_json::Value,
) -> Result<serde_json::Value> {
    let mut request = agent(url).post(url).set("content-type", "application/json");
    for (name, value) in headers {
        request = request.set(name, value);
    }
    match request.send_json(body) {
        Ok(response) => Ok(response.into_json()?),
        Err(ureq::Error::Status(code, response)) => {
            let text = response.into_string().unwrap_or_default();
            Err(anyhow!(
                "POST {url} returned {code}: {}",
                text.chars().take(600).collect::<String>()
            ))
        }
        Err(error) => Err(anyhow!("POST {url}: {error}")),
    }
}

pub(crate) fn env_key(name: &str) -> Result<String> {
    std::env::var(name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| anyhow!("set {name} to use this judge"))
}

#[cfg(test)]
mod tests {
    use super::bypasses_proxy;

    #[test]
    fn loopback_skips_the_proxy() {
        assert!(bypasses_proxy("http://localhost:11434/v1/chat/completions"));
        assert!(bypasses_proxy("http://127.0.0.1:8765/v1"));
        assert!(bypasses_proxy("http://[::1]:8000/v1"));
        assert!(!bypasses_proxy(
            "https://api.openai.com/v1/chat/completions"
        ));
    }
}
