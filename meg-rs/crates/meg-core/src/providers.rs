//! Public provider configuration is separate from OS-protected credentials.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use zeroize::Zeroize;

#[derive(Clone, Default)]
pub struct Secret(String);
impl Secret {
    pub fn new(value: String) -> Self {
        Self(value)
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn push(&mut self, ch: char) {
        self.0.push(ch);
    }
    pub fn pop(&mut self) {
        self.0.pop();
    }
    pub fn replace(&mut self, value: String) {
        self.0.zeroize();
        self.0 = value;
    }
    pub fn insert_str(&mut self, byte: usize, text: &str) {
        self.0.insert_str(byte, text);
    }
    pub fn erase(&mut self, start: usize, end: usize) {
        self.0.replace_range(start..end, "");
    }
}
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}
impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    #[default]
    OpenAi,
    OpenAiResponses,
    Anthropic,
    Gemini,
    Ollama,
    Shim,
}
impl ProviderKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAI compatible",
            Self::OpenAiResponses => "OpenAI Responses API",
            Self::Anthropic => "Anthropic",
            Self::Gemini => "Google Gemini (native)",
            Self::Ollama => "Ollama / local",
            Self::Shim => "Custom compatible",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderProfile {
    pub id: String,
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub default_model: String,
    #[serde(default)]
    pub key_env: Option<String>,
    #[serde(default)]
    pub send_effort: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_scope: Option<String>,
}
impl ProviderProfile {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            !self.id.is_empty()
                && self.id.len() <= 80
                && self
                    .id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')),
            "Invalid provider identifier"
        );
        anyhow::ensure!(
            !self.name.trim().is_empty()
                && self.name.len() <= 80
                && !self.name.chars().any(char::is_control),
            "Provider needs a name (up to 80 characters)"
        );
        let url = reqwest::Url::parse(&self.base_url)
            .context("Enter a complete http:// or https:// endpoint URL")?;
        anyhow::ensure!(
            matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
            "Endpoint must use HTTP or HTTPS"
        );
        if url.scheme() == "http"
            && std::env::var("MEG_ALLOW_INSECURE_HTTP").as_deref() != Ok("1")
        {
            let host = url.host_str().unwrap_or("").to_ascii_lowercase();
            let is_loopback = matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1" | "[::1]" | "0.0.0.0")
                || host.ends_with(".localhost");
            anyhow::ensure!(
                is_loopback,
                "Plaintext HTTP is only allowed for local loopback endpoints; use HTTPS for '{host}' (or set MEG_ALLOW_INSECURE_HTTP=1)"
            );
        }
        anyhow::ensure!(
            url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "Put the API key in its protected field, not in the endpoint URL"
        );
        anyhow::ensure!(
            self.default_model.len() <= 256 && !self.default_model.chars().any(char::is_control),
            "Invalid model identifier"
        );
        if let Some(scope) = &self.credential_scope {
            anyhow::ensure!(
                scope.len() <= 2048 && !scope.chars().any(char::is_control),
                "Invalid credential namespace"
            );
        }
        if let Some(env) = &self.key_env {
            anyhow::ensure!(
                !env.is_empty()
                    && env.len() <= 128
                    && env.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "Invalid API-key environment variable name"
            );
        }
        Ok(())
    }
    pub fn environment() -> Self {
        Self {
            id: "environment".into(),
            name: "Environment endpoint".into(),
            kind: ProviderKind::OpenAi,
            base_url: crate::shim_client::default_base_url(),
            default_model: crate::shim_client::default_model(),
            key_env: std::env::var("MEG_TUI_API_KEY")
                .ok()
                .filter(|s| !s.is_empty())
                .map(|_| "MEG_TUI_API_KEY".into()),
            send_effort: false,
            credential_scope: credential_scope(&config_dir()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSettings {
    pub version: u32,
    pub active: String,
    pub profiles: Vec<ProviderProfile>,
}
impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            version: 1,
            active: "ollama".into(),
            profiles: [
                (
                    "ollama",
                    "Ollama",
                    ProviderKind::Ollama,
                    "http://127.0.0.1:11434/v1",
                ),
                (
                    "lmstudio",
                    "LM Studio",
                    ProviderKind::OpenAi,
                    "http://127.0.0.1:1234/v1",
                ),
                (
                    "openrouter",
                    "OpenRouter",
                    ProviderKind::OpenAi,
                    "https://openrouter.ai/api/v1",
                ),
                (
                    "openai",
                    "OpenAI",
                    ProviderKind::OpenAi,
                    "https://api.openai.com/v1",
                ),
                (
                    "anthropic",
                    "Anthropic",
                    ProviderKind::Anthropic,
                    "https://api.anthropic.com/v1",
                ),
            ]
            .into_iter()
            .map(|(id, name, kind, url)| ProviderProfile {
                id: id.into(),
                name: name.into(),
                kind,
                base_url: url.into(),
                default_model: String::new(),
                key_env: None,
                send_effort: false,
                credential_scope: credential_scope(&config_dir()),
            })
            .collect(),
        }
    }
}

/// Distinctive key prefixes identify a provider, never the key's model access.
/// Generic sk-* keys are deliberately ambiguous and are not sent to guessed hosts.
pub fn detect_key_provider(key: &str) -> Option<(&'static str, ProviderKind, &'static str)> {
    crate::provider_catalog::key_preset(key).map(|p| (p.name, p.kind, p.url))
}

#[derive(Debug, Clone)]
pub struct ProviderStore {
    pub root: PathBuf,
}
impl ProviderStore {
    pub fn global() -> Self {
        Self { root: config_dir() }
    }
    pub fn load(&self) -> Result<ProviderSettings> {
        let path = self.root.join("providers.json");
        let mut settings: ProviderSettings = if path.exists() {
            anyhow::ensure!(
                path.metadata()?.len() <= 256 * 1024,
                "Provider settings exceed 256 KiB"
            );
            serde_json::from_slice(&std::fs::read(path)?)?
        } else {
            Default::default()
        };
        // Retire only the old generated preset identity. Custom profiles
        // and explicitly configured environment endpoints remain operator-owned.
        settings.profiles.retain(|p| {
            !(p.id == "local-shim" && p.name == "Local shim" && p.kind == ProviderKind::Shim)
        });
        if settings.active == "local-shim"
            && !settings.profiles.iter().any(|p| p.id == settings.active)
        {
            settings.active = settings
                .profiles
                .iter()
                .find(|p| p.id == "ollama")
                .or_else(|| settings.profiles.first())
                .map(|p| p.id.clone())
                .unwrap_or_default();
        }
        anyhow::ensure!(
            settings.version == 1 && settings.profiles.len() <= 64,
            "Unsupported provider settings"
        );
        let mut ids = std::collections::HashSet::new();
        for profile in &mut settings.profiles {
            profile.credential_scope = credential_scope(&self.root);
            profile.validate()?;
            anyhow::ensure!(
                ids.insert(profile.id.clone()),
                "Duplicate provider identifier"
            );
        }
        Ok(settings)
    }
    pub fn save(&self, settings: &ProviderSettings) -> Result<()> {
        anyhow::ensure!(settings.profiles.len() <= 64, "Provider limit is 64");
        for profile in &settings.profiles {
            profile.validate()?;
        }
        std::fs::create_dir_all(&self.root)?;
        atomic_write(
            &self.root.join("providers.json"),
            &serde_json::to_vec_pretty(settings)?,
        )
    }
    pub fn upsert(
        &self,
        mut profile: ProviderProfile,
        key: Option<&Secret>,
    ) -> Result<ProviderSettings> {
        profile.credential_scope = credential_scope(&self.root);
        profile.validate()?;
        let mut settings = self.load()?;
        anyhow::ensure!(
            settings.profiles.iter().any(|p| p.id == profile.id) || settings.profiles.len() < 64,
            "Provider limit is 64"
        );
        let previous = if key.is_some() {
            vault::get_scoped(&profile.id, profile.credential_scope.as_deref())?
        } else {
            None
        };
        if let Some(key) = key {
            anyhow::ensure!(
                key.expose().len() <= 2048 && !key.expose().chars().any(char::is_control),
                "API key must be a single line, up to 2048 bytes"
            );
            vault::set_scoped(&profile.id, profile.credential_scope.as_deref(), key)?;
        }
        if let Some(existing) = settings.profiles.iter_mut().find(|p| p.id == profile.id) {
            *existing = profile.clone();
        } else {
            settings.profiles.push(profile.clone());
        }
        settings.active = profile.id.clone();
        if let Err(error) = self.save(&settings) {
            if key.is_some() {
                let rollback = match previous {
                    Some(old) => {
                        vault::set_scoped(&profile.id, profile.credential_scope.as_deref(), &old)
                    }
                    None => vault::remove_scoped(&profile.id, profile.credential_scope.as_deref()),
                };
                rollback.context(
                    "Provider save failed and the previous credential could not be restored",
                )?;
            }
            return Err(error);
        }
        Ok(settings)
    }
    pub fn activate(&self, id: &str) -> Result<ProviderSettings> {
        let mut settings = self.load()?;
        anyhow::ensure!(
            settings.profiles.iter().any(|p| p.id == id),
            "Unknown provider"
        );
        settings.active = id.into();
        self.save(&settings)?;
        Ok(settings)
    }
    pub fn remove(&self, id: &str) -> Result<ProviderSettings> {
        let mut settings = self.load()?;
        let previous = settings.clone();
        settings.profiles.retain(|p| p.id != id);
        if settings.active == id {
            settings.active = settings
                .profiles
                .first()
                .map(|p| p.id.clone())
                .unwrap_or_default();
        }
        self.save(&settings)?;
        if let Err(error) = vault::remove_scoped(id, credential_scope(&self.root).as_deref()) {
            self.save(&previous)
                .context("Credential removal failed and provider settings could not be restored")?;
            return Err(error);
        }
        Ok(settings)
    }
}

pub fn config_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("MEG_CONFIG_DIR") {
        return path.into();
    }
    default_config_dir()
}
fn default_config_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("APPDATA") {
        return PathBuf::from(path).join("MEG");
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("meg")
}
/// Alternate configurations must not silently reuse the operator's global keys.
/// The standard configuration retains its historical vault target names.
pub fn credential_scope(root: &Path) -> Option<String> {
    fn normalize(root: &Path) -> String {
        let absolute = if root.is_absolute() {
            root.to_path_buf()
        } else {
            std::env::current_dir().unwrap_or_default().join(root)
        };
        let resolved = absolute
            .canonicalize()
            .or_else(|_| {
                absolute
                    .parent()
                    .and_then(|p| p.canonicalize().ok())
                    .map(|p| p.join(absolute.file_name().unwrap_or_default()))
                    .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))
            })
            .unwrap_or(absolute);
        let mut text = resolved.to_string_lossy().replace(char::from(92), "/");
        if text.starts_with("//?/UNC/") {
            text = format!("//{}", &text[8..]);
        } else if text.starts_with("//?/") {
            text = text[4..].into();
        }
        let text = text.trim_end_matches('/').to_string();
        if cfg!(windows) {
            text.to_lowercase()
        } else {
            text
        }
    }
    let path = normalize(root);
    (path != normalize(&default_config_dir())).then_some(path)
}
pub fn api_key(profile: &ProviderProfile) -> Result<Option<Secret>> {
    if let Some(env) = &profile.key_env {
        return Ok(std::env::var(env)
            .ok()
            .filter(|s| !s.is_empty())
            .map(Secret::new));
    }
    if let Some(secret) = vault::get_scoped(&profile.id, profile.credential_scope.as_deref())? {
        return Ok(Some(secret));
    }
    // Existing saved presets also recognize their standard environment variable.
    // Only the exact preset endpoint may receive it; a custom URL must opt in.
    Ok(crate::provider_catalog::provider_preset(&profile.id)
        .filter(|preset| preset.url.trim_end_matches('/') == profile.base_url.trim_end_matches('/'))
        .filter(|preset| !preset.env.is_empty())
        .and_then(|preset| std::env::var(preset.env).ok())
        .filter(|value| !value.is_empty())
        .map(Secret::new))
}
pub fn has_key(profile: &ProviderProfile) -> bool {
    api_key(profile)
        .ok()
        .flatten()
        .is_some_and(|s| !s.is_empty())
}
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(tmp);
    }
    result
}

#[cfg(windows)]
pub mod vault {
    use super::*;
    use windows_sys::Win32::Security::Credentials::*;
    fn target(id: &str, scope: Option<&str>) -> Result<Vec<u16>> {
        anyhow::ensure!(
            id.len() <= 80
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')),
            "Invalid credential identifier"
        );
        let target = if let Some(scope) = scope {
            anyhow::ensure!(
                scope.len() <= 2048 && !scope.chars().any(char::is_control),
                "Invalid credential namespace"
            );
            format!("MEG/config/{scope}/provider/{id}")
        } else {
            format!("MEG/provider/{id}")
        };
        Ok(target.encode_utf16().chain(Some(0)).collect())
    }
    pub fn get(id: &str) -> Result<Option<Secret>> {
        get_scoped(id, None)
    }
    pub fn get_scoped(id: &str, scope: Option<&str>) -> Result<Option<Secret>> {
        let name = target(id, scope)?;
        let mut credential: *mut CREDENTIALW = std::ptr::null_mut();
        unsafe {
            if CredReadW(name.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) == 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(1168) {
                    return Ok(None);
                }
                return Err(anyhow::anyhow!(
                    "Windows Credential Manager could not read this connection ({})",
                    error.raw_os_error().unwrap_or(0)
                ));
            }
            let size = (*credential).CredentialBlobSize as usize;
            if size > 2048 {
                CredFree(credential.cast());
                anyhow::bail!("Stored credential exceeds 2048 bytes");
            }
            let bytes = if size == 0 {
                Vec::new()
            } else if (*credential).CredentialBlob.is_null() {
                CredFree(credential.cast());
                anyhow::bail!("Stored credential is invalid");
            } else {
                std::slice::from_raw_parts((*credential).CredentialBlob, size).to_vec()
            };
            let value = String::from_utf8(bytes);
            CredFree(credential.cast());
            Ok(Some(Secret::new(
                value.context("Stored credential is invalid UTF-8")?,
            )))
        }
    }
    pub fn set(id: &str, key: &Secret) -> Result<()> {
        set_scoped(id, None, key)
    }
    pub fn set_scoped(id: &str, scope: Option<&str>, key: &Secret) -> Result<()> {
        if key.is_empty() {
            return remove_scoped(id, scope);
        }
        let mut name = target(id, scope)?;
        let mut user: Vec<u16> = "MEG".encode_utf16().chain(Some(0)).collect();
        let mut credential: CREDENTIALW = unsafe { std::mem::zeroed() };
        credential.Type = CRED_TYPE_GENERIC;
        credential.TargetName = name.as_mut_ptr();
        credential.CredentialBlobSize = key.expose().len() as u32;
        credential.CredentialBlob = key.expose().as_ptr() as *mut u8;
        credential.Persist = CRED_PERSIST_LOCAL_MACHINE;
        credential.UserName = user.as_mut_ptr();
        anyhow::ensure!(
            unsafe { CredWriteW(&credential, 0) } != 0,
            "Windows Credential Manager could not save this API key"
        );
        Ok(())
    }
    pub fn remove(id: &str) -> Result<()> {
        remove_scoped(id, None)
    }
    pub fn remove_scoped(id: &str, scope: Option<&str>) -> Result<()> {
        let name = target(id, scope)?;
        if unsafe { CredDeleteW(name.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 {
            let error = std::io::Error::last_os_error();
            anyhow::ensure!(
                error.raw_os_error() == Some(1168),
                "Windows Credential Manager could not remove this API key"
            );
        }
        Ok(())
    }
}
#[cfg(not(windows))]
pub mod vault {
    use super::*;
    use std::collections::BTreeMap;
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::sync::{Mutex, OnceLock};

    fn vault_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn target(id: &str, scope: Option<&str>) -> Result<String> {
        anyhow::ensure!(
            !id.is_empty()
                && id.len() <= 80
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')),
            "Invalid credential identifier"
        );
        let name = if let Some(scope) = scope {
            anyhow::ensure!(
                scope.len() <= 2048 && !scope.chars().any(char::is_control),
                "Invalid credential namespace"
            );
            format!("MEG/config/{scope}/provider/{id}")
        } else {
            format!("MEG/provider/{id}")
        };
        Ok(name)
    }

    fn force_file_backend() -> bool {
        std::env::var("MEG_VAULT_BACKEND")
            .ok()
            .is_some_and(|v| v.eq_ignore_ascii_case("file"))
    }

    #[cfg(target_os = "macos")]
    fn escape_security_arg(s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('"');
        for ch in s.chars() {
            match ch {
                '\\' | '"' => {
                    out.push('\\');
                    out.push(ch);
                }
                _ => out.push(ch),
            }
        }
        out.push('"');
        out
    }

    #[cfg(target_os = "macos")]
    fn os_get(service: &str) -> Option<Secret> {
        if force_file_backend() || !Path::new("/usr/bin/security").is_file() {
            return None;
        }
        let out = Command::new("/usr/bin/security")
            .args(["find-generic-password", "-a", "MEG", "-s", service, "-w"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let val = String::from_utf8(out.stdout).ok()?;
        let trimmed = val.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() || trimmed.len() > 2048 {
            return None;
        }
        Some(Secret::new(trimmed.to_string()))
    }

    #[cfg(target_os = "macos")]
    fn os_set(service: &str, key: &Secret) -> bool {
        if force_file_backend() || !Path::new("/usr/bin/security").is_file() {
            return false;
        }
        let Ok(mut child) = Command::new("/usr/bin/security")
            .arg("-i")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
        else {
            return false;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let line = format!(
                "add-generic-password -U -a MEG -s {} -w {}\n",
                escape_security_arg(service),
                escape_security_arg(key.expose())
            );
            if stdin.write_all(line.as_bytes()).is_err() {
                let _ = child.kill();
                return false;
            }
        }
        let Ok(out) = child.wait_with_output() else {
            return false;
        };
        if !out.status.success() || !out.stderr.is_empty() {
            return false;
        }
        os_get(service).is_some_and(|stored| stored.expose() == key.expose())
    }

    #[cfg(target_os = "macos")]
    fn os_remove(service: &str) {
        if force_file_backend() || !Path::new("/usr/bin/security").is_file() {
            return;
        }
        let _ = Command::new("/usr/bin/security")
            .args(["delete-generic-password", "-a", "MEG", "-s", service])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    #[cfg(target_os = "linux")]
    fn has_secret_service() -> bool {
        !force_file_backend() && std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some()
    }

    #[cfg(target_os = "linux")]
    fn os_get(service: &str) -> Option<Secret> {
        if !has_secret_service() {
            return None;
        }
        let out = Command::new("secret-tool")
            .args(["lookup", "service", service, "account", "MEG"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let val = String::from_utf8(out.stdout).ok()?;
        let trimmed = val.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() || trimmed.len() > 2048 {
            return None;
        }
        Some(Secret::new(trimmed.to_string()))
    }

    #[cfg(target_os = "linux")]
    fn os_set(service: &str, key: &Secret) -> bool {
        if !has_secret_service() {
            return false;
        }
        let Ok(mut child) = Command::new("secret-tool")
            .args([
                "store",
                "--label=MEG Provider Key",
                "service",
                service,
                "account",
                "MEG",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            return false;
        };
        if let Some(mut stdin) = child.stdin.take() {
            if stdin.write_all(key.expose().as_bytes()).is_err() {
                let _ = child.kill();
                return false;
            }
        }
        child.wait().map(|s| s.success()).unwrap_or(false)
    }

    #[cfg(target_os = "linux")]
    fn os_remove(service: &str) {
        if !has_secret_service() {
            return;
        }
        let _ = Command::new("secret-tool")
            .args(["clear", "service", service, "account", "MEG"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn os_get(_: &str) -> Option<Secret> {
        None
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn os_set(_: &str, _: &Secret) -> bool {
        false
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn os_remove(_: &str) {}

    fn vault_file_path() -> PathBuf {
        config_dir().join("vault.json")
    }

    fn read_file_vault() -> Result<BTreeMap<String, String>> {
        let path = vault_file_path();
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(e) => return Err(e.into()),
        };
        anyhow::ensure!(
            !meta.file_type().is_symlink(),
            "Refusing to read symlinked vault file"
        );
        anyhow::ensure!(meta.len() <= 512 * 1024, "Vault file exceeds 512 KiB");
        let bytes = std::fs::read(&path)?;
        Ok(serde_json::from_slice(&bytes).unwrap_or_default())
    }

    fn write_file_vault(map: &BTreeMap<String, String>) -> Result<()> {
        let path = vault_file_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
            }
        }
        if let Ok(meta) = std::fs::symlink_metadata(&path) {
            anyhow::ensure!(
                !meta.file_type().is_symlink(),
                "Refusing to overwrite symlinked vault file"
            );
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let tmp = path.with_file_name(format!(".vault.{}.{}.tmp", std::process::id(), nanos));
        let payload = serde_json::to_vec_pretty(map)?;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(&tmp)?;
        file.write_all(&payload)?;
        file.sync_all()?;
        drop(file);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn get(id: &str) -> Result<Option<Secret>> {
        get_scoped(id, None)
    }

    pub fn get_scoped(id: &str, scope: Option<&str>) -> Result<Option<Secret>> {
        let service = target(id, scope)?;
        if let Some(secret) = os_get(&service) {
            return Ok(Some(secret));
        }
        let _guard = vault_lock().lock().unwrap_or_else(|e| e.into_inner());
        let map = read_file_vault()?;
        if let Some(val) = map.get(&service) {
            anyhow::ensure!(val.len() <= 2048, "Stored credential exceeds 2048 bytes");
            return Ok(Some(Secret::new(val.clone())));
        }
        Ok(None)
    }

    pub fn set(id: &str, key: &Secret) -> Result<()> {
        set_scoped(id, None, key)
    }

    pub fn set_scoped(id: &str, scope: Option<&str>, key: &Secret) -> Result<()> {
        if key.is_empty() {
            return remove_scoped(id, scope);
        }
        anyhow::ensure!(
            key.expose().len() <= 2048 && !key.expose().chars().any(char::is_control),
            "API key must be a single line, up to 2048 bytes"
        );
        let service = target(id, scope)?;
        if os_set(&service, key) {
            let _guard = vault_lock().lock().unwrap_or_else(|e| e.into_inner());
            if let Ok(mut map) = read_file_vault() {
                if map.remove(&service).is_some() {
                    let _ = write_file_vault(&map);
                }
            }
            return Ok(());
        }
        let _guard = vault_lock().lock().unwrap_or_else(|e| e.into_inner());
        let mut map = read_file_vault()?;
        map.insert(service, key.expose().to_string());
        write_file_vault(&map)
    }

    pub fn remove(id: &str) -> Result<()> {
        remove_scoped(id, None)
    }

    pub fn remove_scoped(id: &str, scope: Option<&str>) -> Result<()> {
        let service = target(id, scope)?;
        os_remove(&service);
        let _guard = vault_lock().lock().unwrap_or_else(|e| e.into_inner());
        let mut map = read_file_vault()?;
        if map.remove(&service).is_some() {
            write_file_vault(&map)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retired_generated_gateway_is_removed_but_custom_endpoints_are_preserved() {
        let root = std::env::temp_dir().join(format!(
            "meg-provider-test-migration-{}",
            uuid::Uuid::new_v4()
        ));
        let store = ProviderStore { root };
        let mut settings = ProviderSettings::default();
        let mut legacy = ProviderProfile::environment();
        legacy.id = "local-shim".into();
        legacy.name = "Local shim".into();
        legacy.kind = ProviderKind::Shim;
        legacy.base_url = "http://127.0.0.1:9911/v1".into();
        let mut personal = legacy.clone();
        personal.id = "personal".into();
        settings.profiles.extend([legacy, personal]);
        settings.active = "local-shim".into();
        store.save(&settings).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.active, "ollama");
        assert!(!loaded.profiles.iter().any(|p| p.id == "local-shim"));
        assert!(loaded.profiles.iter().any(|p| p.id == "personal"));
        std::fs::remove_dir_all(&store.root).unwrap();
    }
    #[test]
    fn migration_preserves_custom_profile_id_name_kind_and_environment_endpoint() {
        for customized in ["id", "name", "kind", "environment"] {
            let store = ProviderStore {
                root: std::env::temp_dir().join(format!(
                    "meg-provider-custom-migration-{}",
                    uuid::Uuid::new_v4()
                )),
            };
            let mut profile = ProviderProfile::environment();
            profile.id = "local-shim".into();
            profile.name = "Local shim".into();
            profile.kind = ProviderKind::Shim;
            profile.base_url = "http://127.0.0.1:9911/v1".into();
            match customized {
                "id" => profile.id = "custom-endpoint".into(),
                "name" => profile.name = "My endpoint".into(),
                "kind" => profile.kind = ProviderKind::OpenAi,
                "environment" => {
                    profile.id = "environment".into();
                    profile.name = "Environment endpoint".into();
                    profile.kind = ProviderKind::OpenAi;
                }
                _ => unreachable!(),
            }
            let mut settings = ProviderSettings::default();
            settings.active = profile.id.clone();
            settings.profiles.push(profile.clone());
            store.save(&settings).unwrap();
            let loaded = store.load().unwrap();
            assert_eq!(loaded.active, profile.id);
            let restored = loaded.profiles.iter().find(|p| p.id == profile.id).unwrap();
            assert_eq!(restored.name, profile.name);
            assert_eq!(restored.kind, profile.kind);
            assert_eq!(restored.base_url, profile.base_url);
            std::fs::remove_dir_all(&store.root).unwrap();
        }
    }
    #[test]
    fn defaults_are_portable_and_distinctive_prefixes_do_not_guess_generic_keys() {
        let settings = ProviderSettings::default();
        assert_eq!(settings.active, "ollama");
        assert!(settings.profiles.iter().any(|p| p.id == "openrouter"));
        assert!(settings.profiles.iter().all(|p| p.default_model.is_empty()));
        assert_eq!(
            settings
                .profiles
                .iter()
                .find(|p| p.id == "ollama")
                .unwrap()
                .base_url,
            "http://127.0.0.1:11434/v1"
        );
        assert_eq!(
            detect_key_provider("sk-or-v1-fixture").unwrap().0,
            "OpenRouter"
        );
        assert!(detect_key_provider("sk-ambiguous-fixture").is_none());
    }
    #[cfg(unix)]
    #[test]
    fn unix_vault_round_trip_enforces_0600_and_cleans_up() {
        use std::os::unix::fs::PermissionsExt;
        let id = uuid::Uuid::new_v4().to_string();
        let key = Secret::new("offline-unix-vault-fixture".into());
        vault::set(&id, &key).unwrap();
        let fetched = vault::get(&id).unwrap().expect("stored key must round-trip");
        assert_eq!(fetched.expose(), key.expose());
        let vault_file = config_dir().join("vault.json");
        if vault_file.is_file() {
            let mode = std::fs::metadata(&vault_file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "fallback vault.json must be strictly 0600");
        }
        vault::remove(&id).unwrap();
        assert!(vault::get(&id).unwrap().is_none());
    }
    #[cfg(windows)]
    #[test]
    fn windows_vault_round_trip_uses_unique_credential_and_cleans_up() {
        let id = uuid::Uuid::new_v4().to_string();
        let key = Secret::new("offline-vault-fixture".into());
        vault::set(&id, &key).unwrap();
        let result = vault::get(&id);
        let cleanup = vault::remove(&id);
        assert_eq!(result.unwrap().unwrap().expose(), key.expose());
        cleanup.unwrap();
        assert!(vault::get(&id).unwrap().is_none());
    }
    #[test]
    fn secrets_are_redacted_and_never_part_of_provider_json() {
        let key = Secret::new("fixture-secret-never-log".into());
        assert!(!format!("{key:?}").contains("fixture-secret"));
        let profile = ProviderProfile::environment();
        assert!(!serde_json::to_string(&profile)
            .unwrap()
            .contains(key.expose()));
    }
    #[test]
    fn endpoint_credentials_and_malformed_profiles_are_rejected() {
        let mut profile = ProviderProfile::environment();
        profile.base_url = "https://user:secret@example.com/v1".into();
        assert!(profile.validate().is_err());
        profile.base_url = "https://example.com/v1?api_key=secret".into();
        assert!(profile.validate().is_err());
        profile.base_url = "http://127.0.0.1:1/v1".into();
        assert!(profile.validate().is_ok());
    }
    #[test]
    fn profiles_round_trip_and_remove_without_storing_keys_in_json() {
        let root = std::env::temp_dir().join(format!("meg-provider-test-{}", uuid::Uuid::new_v4()));
        let store = ProviderStore { root };
        let mut p = ProviderProfile::environment();
        p.id = uuid::Uuid::new_v4().to_string();
        p.key_env = None;
        p.credential_scope = credential_scope(&store.root);
        let s = store.upsert(p.clone(), None).unwrap();
        assert_eq!(s.active, p.id);
        assert_eq!(store.load().unwrap().profiles.last(), Some(&p));
        let removed = store.remove(&p.id).unwrap();
        assert!(!removed.profiles.iter().any(|v| v.id == p.id));
    }
}

// ===========================================================================
// Phase 5C-2: Cross-Provider Role Routing (`provider_id:model_id`)
// ===========================================================================

#[derive(Debug, Clone)]
pub struct ResolvedRoleTarget {
    pub provider: Option<ProviderProfile>,
    pub model: String,
}

/// Read comma/semicolon-separated disabled provider IDs from `MEG_DISABLED_PROVIDERS`.
pub fn disabled_providers_from_env() -> Vec<String> {
    std::env::var("MEG_DISABLED_PROVIDERS")
        .unwrap_or_default()
        .split([',', ';'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .collect()
}

/// Resolve a role model string (`provider_id:model_id` or bare `model_id`) against
/// configured provider profiles while enforcing `disabled_providers` governance.
///
/// Only splits on the first `:` when the prefix matches `default_provider.id` or a
/// configured `ProviderProfile::id`, preserving bare Ollama tags (`llama3:8b`) and
/// OpenRouter slugs (`anthropic/claude-sonnet-4`).
pub fn resolve_role_target(
    spec: &str,
    default_provider: Option<&ProviderProfile>,
    settings: &ProviderSettings,
    disabled_providers: &[String],
) -> Result<ResolvedRoleTarget> {
    let trimmed = spec.trim();
    let (resolved_provider, resolved_model) = if let Some((prefix, rest)) = trimmed.split_once(':')
    {
        let p_trim = prefix.trim();
        let m_trim = rest.trim();
        if !p_trim.is_empty() && !m_trim.is_empty() {
            if let Some(def) = default_provider.filter(|p| p.id.eq_ignore_ascii_case(p_trim)) {
                (Some(def.clone()), m_trim.to_string())
            } else if let Some(found) = settings
                .profiles
                .iter()
                .find(|p| p.id.eq_ignore_ascii_case(p_trim))
            {
                (Some(found.clone()), m_trim.to_string())
            } else {
                (default_provider.cloned(), trimmed.to_string())
            }
        } else {
            (default_provider.cloned(), trimmed.to_string())
        }
    } else {
        (default_provider.cloned(), trimmed.to_string())
    };

    if let Some(prof) = &resolved_provider {
        if disabled_providers
            .iter()
            .any(|d| d.eq_ignore_ascii_case(&prof.id))
        {
            anyhow::bail!(
                "PROVIDER_DISABLED: role route '{trimmed}' targets disabled provider '{}'",
                prof.id
            );
        }
    }

    Ok(ResolvedRoleTarget {
        provider: resolved_provider,
        model: resolved_model,
    })
}

/// Resolve a role model spec using the global `ProviderStore` and `MEG_DISABLED_PROVIDERS`.
pub fn resolve_role_target_global(
    spec: &str,
    default_provider: Option<&ProviderProfile>,
) -> Result<ResolvedRoleTarget> {
    let settings = ProviderStore::global().load().unwrap_or_default();
    let disabled = disabled_providers_from_env();
    resolve_role_target(spec, default_provider, &settings, &disabled)
}

// ===========================================================================
// P2-04: Governance Locks (disabledProviders post-pin check & --pinned-model)
// ===========================================================================

#[derive(Debug, Clone, Default)]
pub struct ProviderGovernance {
    pub disabled_providers: Vec<String>,
    pub pinned_model: Option<String>,
    pub zdr_no_log: bool,
}

/// Resolve an effective `(provider_id, model_id)` while enforcing:
/// 1. `--pinned-model` single-model lock (blocks silent switches), and
/// 2. `disabled_providers` evaluated AFTER `--model` pin resolution (fixes oh-my-pi #13079).
pub fn resolve_provider_with_governance(
    requested_provider: &str,
    requested_model: &str,
    gov: &ProviderGovernance,
) -> Result<(String, String)> {
    let effective_model = match &gov.pinned_model {
        Some(pin) if !pin.trim().is_empty() => {
            if !requested_model.is_empty() && requested_model != pin {
                anyhow::bail!(
                    "GOVERNANCE_LOCK: model '{}' rejected because --pinned-model is locked to '{}'",
                    requested_model,
                    pin
                );
            }
            pin.clone()
        }
        _ => requested_model.to_string(),
    };
    let effective_provider = if let Some((prefix, _)) = effective_model.split_once('/') {
        if requested_provider.is_empty() {
            prefix.to_string()
        } else {
            requested_provider.to_string()
        }
    } else {
        requested_provider.to_string()
    };
    if gov
        .disabled_providers
        .iter()
        .any(|d| d.eq_ignore_ascii_case(&effective_provider))
    {
        anyhow::bail!(
            "GOVERNANCE_LOCK: provider '{}' is disabled (checked after --model pin resolution)",
            effective_provider
        );
    }
    Ok((effective_provider, effective_model))
}

#[cfg(test)]
mod p2_04_governance_tests {
    use super::*;

    #[test]
    fn disabled_providers_enforced_after_pin_resolution_and_pinned_model_locks() {
        let gov = ProviderGovernance {
            disabled_providers: vec!["openai".into()],
            pinned_model: Some("openai/gpt-4o".into()),
            zdr_no_log: true,
        };
        // Even with --pinned-model, disabled provider is caught AFTER pin resolution.
        let err = resolve_provider_with_governance("", "openai/gpt-4o", &gov).unwrap_err();
        assert!(err.to_string().contains("disabled"), "{err}");

        // Switching away from pinned model is rejected.
        let gov2 = ProviderGovernance {
            disabled_providers: vec![],
            pinned_model: Some("anthropic/claude-sonnet".into()),
            zdr_no_log: false,
        };
        assert!(resolve_provider_with_governance("anthropic", "anthropic/claude-sonnet", &gov2).is_ok());
        assert!(resolve_provider_with_governance("anthropic", "deepseek/r1", &gov2).is_err());
    }

    #[test]
    fn remote_plaintext_http_provider_is_rejected_while_loopback_is_allowed() {
        let mut remote = ProviderProfile::environment();
        remote.base_url = "http://api.example.com/v1".into();
        let err = remote.validate().expect_err("remote plaintext HTTP must be blocked");
        assert!(err.to_string().contains("Plaintext HTTP"), "{err}");

        let mut local = ProviderProfile::environment();
        local.base_url = "http://127.0.0.1:11434/v1".into();
        assert!(local.validate().is_ok());
    }

    #[test]
    fn cross_provider_role_routing_resolves_profiles_preserves_ollama_tags_and_enforces_disabled() {
        let settings = ProviderSettings::default();
        let openrouter = settings
            .profiles
            .iter()
            .find(|p| p.id == "openrouter")
            .unwrap()
            .clone();

        // Cross-provider Anthropic route
        let anth = resolve_role_target("anthropic:claude-sonnet-4", Some(&openrouter), &settings, &[])
            .unwrap();
        assert_eq!(anth.provider.as_ref().unwrap().id, "anthropic");
        assert_eq!(anth.model, "claude-sonnet-4");

        // Cross-provider Ollama route preserving model tag colon
        let oll = resolve_role_target("ollama:qwen2.5-coder:32b", Some(&openrouter), &settings, &[])
            .unwrap();
        assert_eq!(oll.provider.as_ref().unwrap().id, "ollama");
        assert_eq!(oll.model, "qwen2.5-coder:32b");

        // Bare Ollama tag does NOT falsely split on ':'
        let bare_tag = resolve_role_target("llama3:8b", Some(&openrouter), &settings, &[]).unwrap();
        assert_eq!(bare_tag.provider.as_ref().unwrap().id, "openrouter");
        assert_eq!(bare_tag.model, "llama3:8b");

        // OpenRouter slash slug stays on default provider
        let slug = resolve_role_target("anthropic/claude-sonnet-4", Some(&openrouter), &settings, &[])
            .unwrap();
        assert_eq!(slug.provider.as_ref().unwrap().id, "openrouter");
        assert_eq!(slug.model, "anthropic/claude-sonnet-4");

        // Disabled provider blocks cross-provider route
        let blocked = resolve_role_target(
            "openai:gpt-5-codex",
            Some(&openrouter),
            &settings,
            &["openai".to_string()],
        )
        .expect_err("disabled provider must block role route");
        assert!(blocked.to_string().contains("PROVIDER_DISABLED"), "{blocked}");
    }
}
