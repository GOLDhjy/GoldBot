use anyhow::Result;

mod glm;
mod kimi;
mod mimo;
mod minimax;

use self::{
    glm::{GlmProvider, base_url_from_env},
    kimi::KimiProvider,
    mimo::MimoProvider,
    minimax::MiniMaxProvider,
};

// ── Conversation message types ────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, Default)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

#[allow(async_fn_in_trait)]
pub(crate) trait LlmProvider {
    async fn chat_stream_with<F, G>(
        &self,
        client: &reqwest::Client,
        messages: &[Message],
        model: &str,
        show_thinking: bool,
        on_delta: F,
        on_thinking_delta: G,
    ) -> Result<(String, Usage)>
    where
        F: FnMut(&str),
        G: FnMut(&str);
}

// ── HTTP client ───────────────────────────────────────────────────────────────

pub fn build_http_client() -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder();

    if let Ok(proxy_url) = std::env::var("HTTP_PROXY") {
        builder = builder.proxy(reqwest::Proxy::all(&proxy_url)?);
    }

    if let Ok(ms) = std::env::var("API_TIMEOUT_MS") {
        if let Ok(ms) = ms.parse::<u64>() {
            builder = builder
                .timeout(std::time::Duration::from_millis(ms))
                .connect_timeout(std::time::Duration::from_secs(10));
        }
    }

    builder.build().map_err(Into::into)
}

// ── Backend / model presets ───────────────────────────────────────────────────

/// 所有可用后端及其模型列表，用于 /model 选择器。
/// 格式：(backend_label, &[model_name, ...])
const GLM_MODEL_PRESETS: &[&str] = &["glm-5", "glm-5.1", "glm-5v-turbo"];

pub const BACKEND_PRESETS: &[(&str, &[&str])] = &[
    ("GLM", GLM_MODEL_PRESETS),
    ("Kimi", &["kimi-for-coding"]),
    ("Mimo", &["mimo-v2-pro", "mimo-v2-flash", "mimo-v2-omni"]),
    (
        "MiniMax",
        &[
            "MiniMax-M2.7",
            "MiniMax-M2.7-highspeed",
            "MiniMax-M2.5",
            "MiniMax-M2.5-highspeed",
        ],
    ),
];

const DEFAULT_GLM_CONTEXT_WINDOW_TOKENS: u32 = 200_000;
const DEFAULT_KIMI_CONTEXT_WINDOW_TOKENS: u32 = 256_000;
const DEFAULT_MIMO_CONTEXT_WINDOW_TOKENS: u32 = 256_000;
const DEFAULT_MINIMAX_CONTEXT_WINDOW_TOKENS: u32 = 204_800;

fn default_kimi_model() -> String {
    let explicit_base = std::env::var("KIMI_BASE_URL").unwrap_or_default();
    if explicit_base.contains("api.kimi.com/coding") {
        return "kimi-for-coding".to_string();
    }
    let key = std::env::var("KIMI_API_KEY").unwrap_or_default();
    if key.starts_with("sk-kimi-") {
        return "kimi-for-coding".to_string();
    }
    "kimi-k2.5".to_string()
}

fn default_glm_model() -> String {
    std::env::var("BIGMODEL_MODEL")
        .or_else(|_| std::env::var("BIGMODEL_CODING_MODEL"))
        .ok()
        .and_then(|model| normalize_glm_model_name(&model))
        .unwrap_or_else(|| "glm-5".to_string())
}

fn normalize_glm_model_name(model: &str) -> Option<String> {
    match model.trim().to_ascii_lowercase().as_str() {
        "glm-5" => Some("glm-5".to_string()),
        "glm-5.1" => Some("glm-5.1".to_string()),
        "glm-5v-turbo" => Some("glm-5v-turbo".to_string()),
        _ => None,
    }
}

// ── Backend selector ──────────────────────────────────────────────────────────

/// 当前使用的 LLM 后端，内部持有已选定的模型名称。
/// 通过 `LLM_PROVIDER=minimax/glm/kimi/mimo` 显式指定，
/// 或自动检测：优先顺序为 Kimi > MiniMax > Mimo > GLM。
#[derive(Clone)]
pub(crate) enum LlmBackend {
    /// GLM 后端，持有当前选定的模型名。
    Glm(String),
    /// Kimi (Moonshot) 后端，持有当前选定的模型名。
    Kimi(String),
    /// Xiaomi MiMo 普通 Chat 后端，持有当前选定的模型名。
    Mimo(String),
    /// MiniMax 后端，持有当前选定的模型名。
    MiniMax(String),
}

impl LlmBackend {
    pub(crate) fn from_env() -> Self {
        let provider = std::env::var("LLM_PROVIDER")
            .unwrap_or_default()
            .to_lowercase();

        match provider.as_str() {
            "glm-coding" | "glm_coding" | "glmcoding" => {
                let model = default_glm_model();
                LlmBackend::Glm(model)
            }
            "kimi" => {
                let model = std::env::var("KIMI_MODEL").unwrap_or_else(|_| default_kimi_model());
                LlmBackend::Kimi(model)
            }
            "mimo" => {
                let model =
                    std::env::var("MIMO_MODEL").unwrap_or_else(|_| "mimo-v2-pro".to_string());
                LlmBackend::Mimo(model)
            }
            "minimax" => {
                let model =
                    std::env::var("MINIMAX_MODEL").unwrap_or_else(|_| "MiniMax-M2.5".to_string());
                LlmBackend::MiniMax(model)
            }
            "glm" => {
                let model = default_glm_model();
                LlmBackend::Glm(model)
            }
            _ => {
                // 自动检测优先级：Kimi > MiniMax > Mimo > GLM
                if std::env::var("KIMI_API_KEY").is_ok() {
                    let model =
                        std::env::var("KIMI_MODEL").unwrap_or_else(|_| default_kimi_model());
                    LlmBackend::Kimi(model)
                } else if std::env::var("MINIMAX_API_KEY").is_ok()
                    && std::env::var("BIGMODEL_API_KEY").is_err()
                {
                    let model = std::env::var("MINIMAX_MODEL")
                        .unwrap_or_else(|_| "MiniMax-M2.5".to_string());
                    LlmBackend::MiniMax(model)
                } else if std::env::var("MIMO_API_KEY").is_ok()
                    && std::env::var("BIGMODEL_API_KEY").is_err()
                {
                    let model =
                        std::env::var("MIMO_MODEL").unwrap_or_else(|_| "mimo-v2-pro".to_string());
                    LlmBackend::Mimo(model)
                } else {
                    let model = default_glm_model();
                    LlmBackend::Glm(model)
                }
            }
        }
    }

    /// 后端标签，与 `BACKEND_PRESETS` 中的 key 一致。
    pub(crate) fn backend_label(&self) -> &str {
        match self {
            Self::Glm(_) => "GLM",
            Self::Kimi(_) => "Kimi",
            Self::Mimo(_) => "Mimo",
            Self::MiniMax(_) => "MiniMax",
        }
    }

    /// 当前选定的模型名。
    pub(crate) fn model_name(&self) -> &str {
        match self {
            Self::Glm(m) | Self::Kimi(m) | Self::Mimo(m) | Self::MiniMax(m) => m,
        }
    }

    pub(crate) fn context_window_tokens(&self) -> u32 {
        env_u32("GOLDBOT_CONTEXT_WINDOW_TOKENS")
            .or_else(|| match self {
                Self::Glm(_) => env_u32("BIGMODEL_CONTEXT_WINDOW_TOKENS")
                    .or_else(|| env_u32("BIGMODEL_CODING_CONTEXT_WINDOW_TOKENS")),
                Self::Kimi(_) => env_u32("KIMI_CONTEXT_WINDOW_TOKENS"),
                Self::Mimo(_) => env_u32("MIMO_CONTEXT_WINDOW_TOKENS"),
                Self::MiniMax(_) => env_u32("MINIMAX_CONTEXT_WINDOW_TOKENS"),
            })
            .unwrap_or_else(|| match self {
                Self::Glm(_) => DEFAULT_GLM_CONTEXT_WINDOW_TOKENS,
                Self::Kimi(_) => DEFAULT_KIMI_CONTEXT_WINDOW_TOKENS,
                Self::Mimo(_) => DEFAULT_MIMO_CONTEXT_WINDOW_TOKENS,
                Self::MiniMax(_) => DEFAULT_MINIMAX_CONTEXT_WINDOW_TOKENS,
            })
    }

    /// 调用 LLM 流式接口，对外隐藏底层 provider 差异。
    pub(crate) async fn chat_stream_with<F, G>(
        &self,
        client: &reqwest::Client,
        messages: &[Message],
        show_thinking: bool,
        on_delta: F,
        on_thinking_delta: G,
    ) -> Result<(String, Usage)>
    where
        F: FnMut(&str),
        G: FnMut(&str),
    {
        match self {
            Self::Glm(model) => {
                GlmProvider
                    .chat_stream_with(
                        client,
                        messages,
                        model,
                        show_thinking,
                        on_delta,
                        on_thinking_delta,
                    )
                    .await
            }
            Self::Kimi(model) => {
                KimiProvider
                    .chat_stream_with(
                        client,
                        messages,
                        model,
                        show_thinking,
                        on_delta,
                        on_thinking_delta,
                    )
                    .await
            }
            Self::Mimo(model) => {
                MimoProvider
                    .chat_stream_with(
                        client,
                        messages,
                        model,
                        show_thinking,
                        on_delta,
                        on_thinking_delta,
                    )
                    .await
            }
            Self::MiniMax(model) => {
                MiniMaxProvider
                    .chat_stream_with(
                        client,
                        messages,
                        model,
                        show_thinking,
                        on_delta,
                        on_thinking_delta,
                    )
                    .await
            }
        }
    }

    /// 返回 (model名, provider主机) 供 UI 启动信息展示。
    pub(crate) fn display_info(&self) -> (String, String) {
        match self {
            Self::Glm(model) => (model.clone(), base_url_from_env()),
            Self::Kimi(model) => {
                let default_base = if std::env::var("KIMI_API_KEY")
                    .unwrap_or_default()
                    .starts_with("sk-kimi-")
                {
                    "https://api.kimi.com/coding/v1"
                } else {
                    "https://api.moonshot.cn/v1"
                };
                (
                    model.clone(),
                    std::env::var("KIMI_BASE_URL").unwrap_or_else(|_| default_base.to_string()),
                )
            }
            Self::MiniMax(model) => (
                model.clone(),
                std::env::var("MINIMAX_BASE_URL")
                    .unwrap_or_else(|_| "https://api.minimaxi.com/v1".to_string()),
            ),
            Self::Mimo(model) => (
                model.clone(),
                std::env::var("MIMO_BASE_URL")
                    .unwrap_or_else(|_| "https://api.xiaomimimo.com/v1".to_string()),
            ),
        }
    }

    /// 当前 provider 所需 API Key 的环境变量名。
    pub(crate) fn required_key_name(&self) -> &'static str {
        match self {
            Self::Glm(_) => "BIGMODEL_API_KEY",
            Self::Kimi(_) => "KIMI_API_KEY",
            Self::Mimo(_) => "MIMO_API_KEY",
            Self::MiniMax(_) => "MINIMAX_API_KEY",
        }
    }
}

fn env_u32(name: &str) -> Option<u32> {
    std::env::var(name).ok()?.trim().parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use super::BACKEND_PRESETS;

    #[test]
    fn glm_backend_presets_include_glm_5_1() {
        let glm_models = BACKEND_PRESETS
            .iter()
            .find(|(label, _)| *label == "GLM")
            .map(|(_, models)| *models)
            .expect("GLM backend preset should exist");

        assert!(glm_models.contains(&"glm-5"));
        assert!(glm_models.contains(&"glm-5.1"));
        assert!(glm_models.contains(&"glm-5v-turbo"));
    }

    #[test]
    fn kimi_backend_presets_include_selectable_models() {
        let kimi_models = BACKEND_PRESETS
            .iter()
            .find(|(label, _)| *label == "Kimi")
            .map(|(_, models)| *models)
            .expect("Kimi backend preset should exist");

        assert_eq!(kimi_models.len(), 1);
        assert!(kimi_models.contains(&"kimi-for-coding"));
    }

    #[test]
    fn minimax_backend_presets_include_current_models() {
        let minimax_models = BACKEND_PRESETS
            .iter()
            .find(|(label, _)| *label == "MiniMax")
            .map(|(_, models)| *models)
            .expect("MiniMax backend preset should exist");

        assert!(minimax_models.contains(&"MiniMax-M2.7"));
        assert!(minimax_models.contains(&"MiniMax-M2.7-highspeed"));
        assert!(minimax_models.contains(&"MiniMax-M2.5"));
        assert!(minimax_models.contains(&"MiniMax-M2.5-highspeed"));
        assert!(!minimax_models.contains(&"MiniMax-M2.1"));
        assert!(!minimax_models.contains(&"MiniMax-M2.1-highspeed"));
        assert!(!minimax_models.contains(&"MiniMax-M2"));
    }

    #[test]
    fn mimo_backend_presets_include_current_models() {
        let mimo_models = BACKEND_PRESETS
            .iter()
            .find(|(label, _)| *label == "Mimo")
            .map(|(_, models)| *models)
            .expect("Mimo backend preset should exist");

        assert!(mimo_models.contains(&"mimo-v2-pro"));
        assert!(mimo_models.contains(&"mimo-v2-flash"));
        assert!(mimo_models.contains(&"mimo-v2-omni"));
    }
}
