use anyhow::Result;

use crate::agent::provider_glm::GlmProvider;
use crate::agent::provider_minimax::MiniMaxProvider;

// ── Conversation message types ────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
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
pub const BACKEND_PRESETS: &[(&str, &[&str])] = &[
    ("GLM", &["GLM-4.7", "glm-5"]),
    ("MiniMax", &["MiniMax-M2.5", "MiniMax-M2.1"]),
];

// ── Backend selector ──────────────────────────────────────────────────────────

/// 当前使用的 LLM 后端，内部持有已选定的模型名称。
/// 通过 `LLM_PROVIDER=minimax/glm` 显式指定，
/// 或自动检测：只有 MINIMAX_API_KEY 时选 MiniMax，否则默认 GLM。
#[derive(Clone)]
pub(crate) enum LlmBackend {
    /// GLM 后端，持有当前选定的模型名。
    Glm(String),
    /// MiniMax 后端，持有当前选定的模型名。
    MiniMax(String),
}

impl LlmBackend {
    pub(crate) fn from_env() -> Self {
        let is_minimax = match std::env::var("LLM_PROVIDER").as_deref() {
            Ok("minimax") => true,
            Ok("glm") => false,
            _ => {
                std::env::var("BIGMODEL_API_KEY").is_err()
                    && std::env::var("MINIMAX_API_KEY").is_ok()
            }
        };
        if is_minimax {
            let model =
                std::env::var("MINIMAX_MODEL").unwrap_or_else(|_| "MiniMax-M2.5".to_string());
            LlmBackend::MiniMax(model)
        } else {
            let model = std::env::var("BIGMODEL_MODEL").unwrap_or_else(|_| "glm-5".to_string());
            LlmBackend::Glm(model)
        }
    }

    /// 后端标签，与 `BACKEND_PRESETS` 中的 key 一致。
    pub(crate) fn backend_label(&self) -> &str {
        match self {
            Self::Glm(_) => "GLM",
            Self::MiniMax(_) => "MiniMax",
        }
    }

    /// 当前选定的模型名。
    pub(crate) fn model_name(&self) -> &str {
        match self {
            Self::Glm(m) | Self::MiniMax(m) => m,
        }
    }

    /// 调用 LLM 流式接口，对外隐藏底层 provider 差异。
    pub(crate) async fn chat_stream_with<F, G>(
        &self,
        client: &reqwest::Client,
        messages: &[Message],
        show_thinking: bool,
        on_delta: F,
        on_thinking_delta: G,
    ) -> Result<String>
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
            Self::Glm(model) => (
                model.clone(),
                std::env::var("BIGMODEL_BASE_URL")
                    .unwrap_or_else(|_| "https://open.bigmodel.cn/api/coding/paas/v4".to_string()),
            ),
            Self::MiniMax(model) => (
                model.clone(),
                std::env::var("MINIMAX_BASE_URL")
                    .unwrap_or_else(|_| "https://api.minimaxi.com/v1".to_string()),
            ),
        }
    }

    /// 当前 provider 所需 API Key 的环境变量名。
    pub(crate) fn required_key_name(&self) -> &'static str {
        match self {
            Self::Glm(_) => "BIGMODEL_API_KEY",
            Self::MiniMax(_) => "MINIMAX_API_KEY",
        }
    }

    /// 检查所需 API Key 是否缺失。
    pub(crate) fn api_key_missing(&self) -> bool {
        std::env::var(self.required_key_name()).is_err()
    }
}
