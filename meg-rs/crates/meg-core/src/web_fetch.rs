//! Native read-only `web_fetch` tool with SSRF protection, manual redirect
//! validation, bounded streaming download, and HTML-to-text extraction.

use anyhow::{bail, Context, Result};
use regex::Regex;
use reqwest::Url;
use serde_json::{json, Value};
use std::net::IpAddr;
use std::sync::OnceLock;
use std::time::Duration;

const DEFAULT_TIMEOUT_SECS: u64 = 15;
const MAX_TIMEOUT_SECS: u64 = 45;
const MAX_REDIRECTS: usize = 3;
const MAX_RAW_BYTES: usize = 256 * 1024;
const DEFAULT_MAX_CHARS: usize = 12_000;
const MAX_ALLOWED_CHARS: usize = 32_000;

pub fn tool_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "url": {
                "type": "string",
                "description": "Public http:// or https:// URL to fetch (private/loopback/metadata IPs are blocked)."
            },
            "max_chars": {
                "type": "integer",
                "description": "Maximum characters of cleaned text to return (default 12000, max 32000)."
            },
            "timeout_seconds": {
                "type": "integer",
                "description": "HTTP timeout in seconds (default 15, max 45)."
            }
        },
        "required": ["url"]
    })
}

pub fn is_blocked_ip(ip: IpAddr, allow_loopback: bool) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback() {
                return !allow_loopback;
            }
            let oct = v4.octets();
            let is_cgnat = oct[0] == 100 && (oct[1] & 0b1100_0000) == 64;
            let is_zero_net = oct[0] == 0;
            v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                || is_cgnat
                || is_zero_net
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_blocked_ip(IpAddr::V4(v4), allow_loopback);
            }
            if v6.is_loopback() {
                return !allow_loopback;
            }
            let seg0 = v6.segments()[0];
            let is_unique_local = (seg0 & 0xfe00) == 0xfc00;
            let is_link_local = (seg0 & 0xffc0) == 0xfe80;
            v6.is_unspecified() || v6.is_multicast() || is_unique_local || is_link_local
        }
    }
}

async fn validate_url_ssrf(raw_url: &str, allow_loopback: bool) -> Result<Url> {
    let parsed = Url::parse(raw_url.trim())
        .with_context(|| format!("invalid URL '{raw_url}'"))?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        bail!("WEB_FETCH_BLOCKED: unsupported scheme '{scheme}' (only http/https allowed)");
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        bail!("WEB_FETCH_BLOCKED: URLs with embedded credentials are not allowed");
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("WEB_FETCH_BLOCKED: URL missing host"))?;
    let host_lc = host.to_ascii_lowercase();
    if !allow_loopback
        && (host_lc == "localhost"
            || host_lc.ends_with(".localhost")
            || host_lc.ends_with(".local")
            || host_lc.ends_with(".internal"))
    {
        bail!("WEB_FETCH_BLOCKED: local/internal hostname '{host}' is blocked");
    }

    if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
        if is_blocked_ip(ip, allow_loopback) {
            bail!("WEB_FETCH_BLOCKED: IP address '{ip}' is private, loopback, or link-local");
        }
        return Ok(parsed);
    }

    let port = parsed.port_or_known_default().unwrap_or(80);
    let addrs = tokio::net::lookup_host((host, port))
        .await
        .with_context(|| format!("DNS lookup failed for '{host}'"))?;
    let mut resolved_any = false;
    for sa in addrs {
        resolved_any = true;
        if is_blocked_ip(sa.ip(), allow_loopback) {
            bail!(
                "WEB_FETCH_BLOCKED: host '{host}' resolved to blocked address '{}'",
                sa.ip()
            );
        }
    }
    if !resolved_any {
        bail!("DNS lookup returned no addresses for '{host}'");
    }
    Ok(parsed)
}

fn strip_html_to_text(html: &str) -> String {
    static RE_BLOCKS: OnceLock<Regex> = OnceLock::new();
    static RE_BREAKS: OnceLock<Regex> = OnceLock::new();
    static RE_TAGS: OnceLock<Regex> = OnceLock::new();
    static RE_SPACES: OnceLock<Regex> = OnceLock::new();
    static RE_BLANKLINES: OnceLock<Regex> = OnceLock::new();

    let re_blocks = RE_BLOCKS.get_or_init(|| {
        Regex::new(r"(?is)<(script|style|noscript|svg|head)\b[^>]*>.*?</(script|style|noscript|svg|head)>")
            .unwrap()
    });
    let re_breaks = RE_BREAKS.get_or_init(|| {
        Regex::new(r"(?i)</?(p|div|br|li|tr|h[1-6]|section|article|header|footer|pre)\b[^>]*>")
            .unwrap()
    });
    let re_tags = RE_TAGS.get_or_init(|| Regex::new(r"(?s)<[^>]+>").unwrap());
    let re_spaces = RE_SPACES.get_or_init(|| Regex::new(r"[ \t\r\f\v]+").unwrap());
    let re_blanklines = RE_BLANKLINES.get_or_init(|| Regex::new(r"\n{3,}").unwrap());

    let no_blocks = re_blocks.replace_all(html, " ");
    let with_breaks = re_breaks.replace_all(&no_blocks, "\n");
    let no_tags = re_tags.replace_all(&with_breaks, " ");
    let decoded = no_tags
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'");

    let collapsed_lines: Vec<String> = decoded
        .lines()
        .map(|line| re_spaces.replace_all(line, " ").trim().to_string())
        .collect();
    re_blanklines
        .replace_all(&collapsed_lines.join("\n"), "\n\n")
        .trim()
        .to_string()
}

fn truncate_chars(text: &str, max_chars: usize) -> (String, bool) {
    let mut iter = text.char_indices();
    if let Some((byte_idx, _)) = iter.nth(max_chars) {
        (text[..byte_idx].to_string(), true)
    } else {
        (text.to_string(), false)
    }
}

/// Fetch a URL safely with SSRF checks on every redirect hop.
pub async fn fetch_url(args: &Value) -> Result<String> {
    let allow_loopback = std::env::var("MEG_WEB_FETCH_ALLOW_LOOPBACK").as_deref() == Ok("1");
    fetch_url_with_policy(args, allow_loopback).await
}

pub async fn fetch_url_with_policy(args: &Value, allow_loopback: bool) -> Result<String> {
    let raw_url = args
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'url'"))?;
    let max_chars = args
        .get("max_chars")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_MAX_CHARS as u64)
        .clamp(256, MAX_ALLOWED_CHARS as u64) as usize;
    let timeout_secs = args
        .get("timeout_seconds")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .clamp(1, MAX_TIMEOUT_SECS);

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(timeout_secs))
        .user_agent("MEG-WebFetch/0.1")
        .build()
        .context("failed to build HTTP client")?;

    let mut current_url = validate_url_ssrf(raw_url, allow_loopback).await?;
    let mut resp = None;
    for hop in 0..=MAX_REDIRECTS {
        let r = client
            .get(current_url.clone())
            .header(
                reqwest::header::ACCEPT,
                "text/markdown, text/plain, text/html, application/json;q=0.9, */*;q=0.5",
            )
            .send()
            .await
            .with_context(|| format!("HTTP GET failed for '{current_url}'"))?;
        if r.status().is_redirection() {
            if hop == MAX_REDIRECTS {
                bail!("WEB_FETCH_BLOCKED: exceeded max redirects ({MAX_REDIRECTS})");
            }
            let loc = r
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| anyhow::anyhow!("redirect missing Location header"))?;
            let next_url = current_url
                .join(loc)
                .with_context(|| format!("invalid redirect target '{loc}'"))?;
            current_url = validate_url_ssrf(next_url.as_str(), allow_loopback).await?;
            continue;
        }
        resp = Some(r);
        break;
    }

    let mut response = resp.ok_or_else(|| anyhow::anyhow!("no HTTP response received"))?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    if !status.is_success() {
        bail!("HTTP {status} fetching '{current_url}'");
    }

    let mut raw_bytes = Vec::new();
    let mut raw_truncated = false;
    while let Some(chunk) = response.chunk().await? {
        let remaining = MAX_RAW_BYTES.saturating_sub(raw_bytes.len());
        if remaining == 0 {
            raw_truncated = true;
            break;
        }
        if chunk.len() > remaining {
            raw_bytes.extend_from_slice(&chunk[..remaining]);
            raw_truncated = true;
            break;
        }
        raw_bytes.extend_from_slice(&chunk);
    }

    let body_raw = String::from_utf8_lossy(&raw_bytes).to_string();
    let ct_lc = content_type.to_ascii_lowercase();
    let cleaned = if ct_lc.contains("html") || body_raw.trim_start().starts_with("<!DOCTYPE html") || body_raw.trim_start().starts_with("<html") {
        strip_html_to_text(&body_raw)
    } else {
        body_raw
    };

    let (excerpt, text_truncated) = truncate_chars(&cleaned, max_chars);
    let trunc_note = if raw_truncated || text_truncated {
        " (truncated)"
    } else {
        ""
    };

    Ok(format!(
        "[web_fetch untrusted external content | status={} | content-type={} | url={}{}]\n{}",
        status.as_u16(),
        content_type,
        current_url,
        trunc_note,
        excerpt
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn blocks_private_loopback_metadata_and_non_http_urls() {
        for bad in [
            "file:///etc/passwd",
            "http://127.0.0.1:8080/secret",
            "http://localhost:11434/v1/models",
            "http://169.254.169.254/latest/meta-data/",
            "http://10.0.0.1/admin",
            "http://192.168.1.1/",
            "http://[::ffff:169.254.169.254]/",
            "https://user:pass@example.com/",
        ] {
            let err = fetch_url_with_policy(&json!({ "url": bad }), false)
                .await
                .expect_err(&format!("expected SSRF block for {bad}"));
            assert!(
                err.to_string().contains("WEB_FETCH_BLOCKED"),
                "unexpected error for {bad}: {err}"
            );
        }
    }

    #[tokio::test]
    async fn fetches_and_cleans_html_and_blocks_private_redirects() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/doc"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/html; charset=utf-8")
                    .set_body_string(
                        "<html><head><title>Ignored</title><style>body{color:red}</style></head>\
                         <body><h1>Hello &amp; Welcome</h1><script>alert(1)</script><p>Rust &lt;3&gt; MEG</p></body></html>",
                    ),
            )
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/redir-meta"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", "http://169.254.169.254/latest/meta-data/"),
            )
            .mount(&server)
            .await;

        let ok = fetch_url_with_policy(
            &json!({ "url": format!("{}/doc", server.uri()) }),
            true,
        )
        .await
        .expect("loopback mock doc fetch should succeed when explicitly enabled");
        assert!(ok.contains("Hello & Welcome"), "{ok}");
        assert!(ok.contains("Rust <3> MEG"), "{ok}");
        assert!(!ok.contains("alert(1)"), "script body leaked: {ok}");
        assert!(!ok.contains("color:red"), "style body leaked: {ok}");

        let redir_err = fetch_url_with_policy(
            &json!({ "url": format!("{}/redir-meta", server.uri()) }),
            true,
        )
        .await
        .expect_err("redirect to AWS metadata IP must be blocked even when loopback is allowed");
        assert!(
            redir_err.to_string().contains("WEB_FETCH_BLOCKED"),
            "{redir_err}"
        );
    }
}
