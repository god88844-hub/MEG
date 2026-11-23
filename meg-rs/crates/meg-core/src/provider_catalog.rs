//! Service metadata is public; credentials remain in providers::vault.
//! Compatible endpoints follow the services' documented Chat Completions APIs.
use crate::providers::{ProviderKind, ProviderProfile};

#[derive(Debug, Clone, Copy)]
pub struct ProviderPreset {
    pub id: &'static str,
    pub name: &'static str,
    pub url: &'static str,
    pub kind: ProviderKind,
    pub env: &'static str,
    pub prefix: &'static str,
    pub local: bool,
}
impl ProviderPreset {
    pub fn profile(self) -> ProviderProfile {
        ProviderProfile {
            id: self.id.into(),
            name: self.name.into(),
            kind: self.kind,
            base_url: self.url.into(),
            default_model: String::new(),
            key_env: (!self.env.is_empty() && std::env::var(self.env).is_ok_and(|v| !v.is_empty()))
                .then(|| self.env.into()),
            send_effort: false,
            credential_scope: crate::providers::credential_scope(&crate::providers::config_dir()),
        }
    }
}
macro_rules! compatible {
    ($id:literal,$name:literal,$url:literal,$env:literal,$prefix:literal) => {
        ProviderPreset {
            id: $id,
            name: $name,
            url: $url,
            env: $env,
            prefix: $prefix,
            kind: ProviderKind::OpenAi,
            local: false,
        }
    };
}
pub const PROVIDER_PRESETS: &[ProviderPreset] = &[
    compatible!(
        "openrouter",
        "OpenRouter",
        "https://openrouter.ai/api/v1",
        "OPENROUTER_API_KEY",
        "sk-or-v1-"
    ),
    compatible!(
        "openai",
        "OpenAI",
        "https://api.openai.com/v1",
        "OPENAI_API_KEY",
        "sk-proj-"
    ),
    ProviderPreset {
        id: "anthropic",
        name: "Anthropic",
        url: "https://api.anthropic.com/v1",
        kind: ProviderKind::Anthropic,
        env: "ANTHROPIC_API_KEY",
        prefix: "sk-ant-",
        local: false,
    },
    compatible!(
        "nvidia",
        "NVIDIA NIM",
        "https://integrate.api.nvidia.com/v1",
        "NVIDIA_API_KEY",
        "nvapi-"
    ),
    compatible!(
        "google",
        "Google Gemini",
        "https://generativelanguage.googleapis.com/v1beta/openai",
        "GEMINI_API_KEY",
        "AIza"
    ),
    compatible!(
        "groq",
        "Groq",
        "https://api.groq.com/openai/v1",
        "GROQ_API_KEY",
        "gsk_"
    ),
    compatible!(
        "deepseek",
        "DeepSeek",
        "https://api.deepseek.com",
        "DEEPSEEK_API_KEY",
        ""
    ),
    compatible!(
        "mistral",
        "Mistral",
        "https://api.mistral.ai/v1",
        "MISTRAL_API_KEY",
        ""
    ),
    compatible!("xai", "xAI", "https://api.x.ai/v1", "XAI_API_KEY", "xai-"),
    compatible!(
        "fireworks",
        "Fireworks AI",
        "https://api.fireworks.ai/inference/v1",
        "FIREWORKS_API_KEY",
        ""
    ),
    compatible!(
        "siliconflow",
        "SiliconFlow",
        "https://api.siliconflow.com/v1",
        "SILICONFLOW_API_KEY",
        ""
    ),
    compatible!(
        "nebius",
        "Nebius Token Factory",
        "https://api.tokenfactory.nebius.com/v1",
        "NEBIUS_API_KEY",
        ""
    ),
    ProviderPreset {
        id: "ollama",
        name: "Ollama",
        url: "http://127.0.0.1:11434/v1",
        kind: ProviderKind::Ollama,
        env: "",
        prefix: "",
        local: true,
    },
    ProviderPreset {
        id: "lmstudio",
        name: "LM Studio",
        url: "http://127.0.0.1:1234/v1",
        kind: ProviderKind::OpenAi,
        env: "",
        prefix: "",
        local: true,
    },
    ProviderPreset {
        id: "llamacpp",
        name: "llama.cpp",
        url: "http://127.0.0.1:8080/v1",
        kind: ProviderKind::OpenAi,
        env: "",
        prefix: "",
        local: true,
    },
    ProviderPreset {
        id: "vllm",
        name: "vLLM",
        url: "http://127.0.0.1:8000/v1",
        kind: ProviderKind::OpenAi,
        env: "",
        prefix: "",
        local: true,
    },
];
pub fn provider_preset(id: &str) -> Option<&'static ProviderPreset> {
    PROVIDER_PRESETS
        .iter()
        .find(|p| p.id.eq_ignore_ascii_case(id) || p.name.eq_ignore_ascii_case(id))
}
pub fn key_preset(key: &str) -> Option<&'static ProviderPreset> {
    PROVIDER_PRESETS
        .iter()
        .find(|p| !p.prefix.is_empty() && key.starts_with(p.prefix))
}
pub fn is_local(profile: &ProviderProfile) -> bool {
    reqwest::Url::parse(&profile.base_url)
        .ok()
        .is_some_and(|u| {
            matches!(
                u.host_str(),
                Some("localhost" | "127.0.0.1" | "::1" | "[::1]")
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_has_valid_unique_profiles_and_never_guesses_opaque_keys() {
        let mut ids = std::collections::HashSet::new();
        for p in PROVIDER_PRESETS {
            p.profile().validate().unwrap();
            assert!(ids.insert(p.id));
        }
        assert_eq!(key_preset("nvapi-fixture-only").unwrap().id, "nvidia");
        assert_eq!(key_preset("gsk_fixture-only").unwrap().id, "groq");
        assert!(key_preset("sk-ambiguous-fixture-only").is_none());
        assert!(key_preset("not-an-api-key").is_none());
    }
}
