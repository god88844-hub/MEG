//! Pure-Rust security router, DDIPE supply-chain scanner, CLI > MCP policy,
//! and lightweight SHA-256 hashing (no extra crate dependencies).

use regex::Regex;
use serde::Serialize;
use std::sync::OnceLock;

const SECURITY_TASKS: &[&str] = &[
    "filter-js-from-html",
    "break-filter-js-from-html",
    "password-recovery",
    "crack-7z-hash",
    "feal-linear-cryptanalysis",
    "feal-differential-cryptanalysis",
    "vulnerable-secret",
    "sanitize-git-repo",
    "git-leak-recovery",
    "fix-code-vulnerability",
    "openssl-selfsigned-cert",
];

fn security_prompt_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b(xss|cross-site|exploit|payload|injection|crack|decrypt|cryptanalysis|secret key|red-team|7z hash|recover the password|password.recovery)\b")
            .expect("valid security regex")
    })
}

pub fn is_security_or_crypto_task(task_name: Option<&str>, prompt: &str) -> bool {
    if let Some(name) = task_name {
        if SECURITY_TASKS.contains(&name) {
            return true;
        }
    }
    security_prompt_re().is_match(prompt)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteVerdict {
    RootThread(String),
    Delegate(String),
}

pub fn route_task(task_name: Option<&str>, prompt: &str, handoffs_so_far: usize) -> RouteVerdict {
    if is_security_or_crypto_task(task_name, prompt) {
        return RouteVerdict::RootThread(
            "security/crypto task: delegation strips benchmark framing (0/14 delegated vs 6/6 root) ? execute in the root thread".into(),
        );
    }
    if handoffs_so_far >= 5 {
        return RouteVerdict::RootThread(
            "hand-off cap (5) reached ? finish in the root thread".into(),
        );
    }
    RouteVerdict::Delegate(format!("hand-off {} of max 5", handoffs_so_far + 1))
}

#[derive(Debug, Clone, Serialize)]
pub struct DdipeFinding {
    pub rule: String,
    pub severity: &'static str,
    pub line: usize,
    pub snippet: String,
    #[serde(rename = "inCodeBlock")]
    pub in_code_block: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DdipeScanReport {
    pub safe: bool,
    pub source: String,
    pub findings: Vec<DdipeFinding>,
}

struct DdipeRule {
    rule: &'static str,
    severity: &'static str,
    re: Regex,
}

fn ddipe_rules() -> &'static [DdipeRule] {
    static RULES: OnceLock<Vec<DdipeRule>> = OnceLock::new();
    RULES.get_or_init(|| {
        vec![
            DdipeRule {
                rule: "pip_conf_hijack",
                severity: "critical",
                re: Regex::new(r"(?i)\bpip\.conf\b|\bpip\.ini\b").unwrap(),
            },
            DdipeRule {
                rule: "pip_extra_index_url",
                severity: "critical",
                re: Regex::new(r"(?i)--extra-index-url|\bextra-index-url\b|\bPIP_EXTRA_INDEX_URL\b").unwrap(),
            },
            DdipeRule {
                rule: "python_requests_exfil",
                severity: "critical",
                re: Regex::new(r"(?i)\brequests\.(post|put)\s*\(").unwrap(),
            },
            DdipeRule {
                rule: "curl_data_exfil",
                severity: "critical",
                re: Regex::new(r"(?i)\bcurl\b[^\n]*(\s-d\s|\s--data\b|\s--data-binary\b|\s-F\s)").unwrap(),
            },
            DdipeRule {
                rule: "ssh_authorized_keys",
                severity: "critical",
                re: Regex::new(r"(?i)\bauthorized_keys\b").unwrap(),
            },
            DdipeRule {
                rule: "ssh_dir_access",
                severity: "critical",
                re: Regex::new(r#"(?i)(^|[\s"'`~/$])\.ssh(/|\\|\b)"#).unwrap(),
            },
            DdipeRule {
                rule: "pipe_to_shell",
                severity: "high",
                re: Regex::new(r"(?i)\b(curl|wget)\b[^\n]*\|\s*(sh|bash|zsh|pwsh|powershell)\b").unwrap(),
            },
            DdipeRule {
                rule: "base64_shell_exec",
                severity: "high",
                re: Regex::new(r"(?i)base64\s+(-d|--decode)\s*\|\s*(sh|bash|python|node)\b").unwrap(),
            },
        ]
    })
}

pub fn scan_skill_supply_chain(content: &str, source: &str) -> DdipeScanReport {
    let mut findings = Vec::new();
    let mut in_code_block = false;
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        for spec in ddipe_rules() {
            if spec.re.is_match(line) {
                let snippet: String = trimmed.chars().take(140).collect();
                findings.push(DdipeFinding {
                    rule: spec.rule.to_string(),
                    severity: spec.severity,
                    line: idx + 1,
                    snippet,
                    in_code_block,
                });
            }
        }
    }
    DdipeScanReport {
        safe: findings.is_empty(),
        source: source.to_string(),
        findings,
    }
}

pub const CLI_FIRST_BINARIES: &[&str] = &["gh", "supabase", "stripe", "vercel", "git", "curl"];
pub const DEFAULT_ALLOWED_MCP_SERVERS: &[&str] = &["chrome-devtools", "Context7", "sentry"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliOrMcpRoute {
    Cli { binary: String, reason: String },
    Mcp { server: String, reason: String },
    Denied { target: String, reason: String },
}

pub fn route_cli_or_mcp(target: &str, extra_allowed: &[String]) -> CliOrMcpRoute {
    let clean = target.trim();
    let low = clean.to_ascii_lowercase();
    for &b in CLI_FIRST_BINARIES {
        if low == b
            || low == format!("{b}-mcp")
            || low.starts_with(&format!("{b}_"))
            || low.starts_with(&format!("{b}-"))
        {
            return CliOrMcpRoute::Cli {
                binary: b.to_string(),
                reason: format!(
                    "CLI > MCP policy: use native '{b}' CLI directly instead of MCP wrapper"
                ),
            };
        }
    }
    let allowed = DEFAULT_ALLOWED_MCP_SERVERS
        .iter()
        .any(|s| s.eq_ignore_ascii_case(clean))
        || extra_allowed.iter().any(|s| s.eq_ignore_ascii_case(clean));
    if allowed {
        return CliOrMcpRoute::Mcp {
            server: clean.to_string(),
            reason: format!("Allowed default MCP server '{clean}'"),
        };
    }
    CliOrMcpRoute::Denied {
        target: clean.to_string(),
        reason: format!(
            "MCP server '{clean}' blocked by default allowlist ({})",
            DEFAULT_ALLOWED_MCP_SERVERS.join(", ")
        ),
    }
}

pub fn sanitize_mcp_alias(server: &str, tool: &str) -> String {
    let clean = |s: &str| -> String {
        let mut out = String::with_capacity(s.len());
        let mut prev_us = false;
        for ch in s.chars() {
            if ch.is_ascii_alphanumeric() || ch == '-' {
                out.push(ch);
                prev_us = false;
            } else if !prev_us {
                out.push('_');
                prev_us = true;
            }
        }
        let trimmed = out.trim_matches('_');
        if trimmed.is_empty() {
            "x".to_string()
        } else {
            trimmed.to_string()
        }
    };
    let raw = format!("mcp__{}__{}", clean(server), clean(tool));
    if raw.len() <= 64 {
        return raw;
    }
    let mut h: u32 = 2_166_136_261;
    for b in raw.bytes() {
        h = (h ^ u32::from(b)).wrapping_mul(16_777_619);
    }
    format!("{}_{:08x}", &raw[..55], h)
}

/// FIPS 180-4 SHA-256 returning lowercase 64-char hex.
pub fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let bit_len = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[4 * i],
                chunk[4 * i + 1],
                chunk[4 * i + 2],
                chunk[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, delta) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(delta);
        }
    }
    let mut hex = String::with_capacity(64);
    for word in h {
        hex.push_str(&format!("{word:08x}"));
    }
    hex
}
