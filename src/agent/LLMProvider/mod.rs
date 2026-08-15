use anyhow::Result;

mod deepseek;
mod glm;
mod kimi;
mod mimo;
mod minimax;

// ── Debug context logger ──────────────────────────────────────────────────────
/// 若 `GOLDBOT_DEBUG_LOG` 非空，则将每次 LLM 调用前的完整消息列表追加写入
/// `~/.goldbot/llm_context.log`，方便排查 Sub-Agent / skill 上下文传递问题。
fn maybe_write_debug_log(messages: &[Message]) {
    if std::env::var("GOLDBOT_DEBUG_LOG")
        .unwrap_or_default()
        .is_empty()
    {
        return;
    }
    let log_path = crate::tools::mcp::goldbot_home_dir().join("llm_context.log");
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut out = format!("\n\n=== LLM CALL @ {timestamp} ===\n");
    for (i, msg) in messages.iter().enumerate() {
        let role = match msg.role {
            Role::System => "SYSTEM",
            Role::User => "USER",
            Role::Assistant => "ASSISTANT",
        };
        out.push_str(&format!("--- [{i}] {role} ---\n{}\n", msg.content));
    }
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .and_then(|mut f| {
            use std::io::Write;
            f.write_all(out.as_bytes())
        });
}

use self::{
    deepseek::DeepSeekProvider,
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
const GLM_MODEL_PRESETS: &[&str] = &["glm-5.3"];
const DEFAULT_MIMO_MODEL: &str = "mimo-v2.5-pro";
const MIMO_MODEL_PRESETS: &[&str] = &[
    DEFAULT_MIMO_MODEL,
    "mimo-v2-pro",
    "mimo-v2-flash",
    "mimo-v2-omni",
];

pub const BACKEND_PRESETS: &[(&str, &[&str])] = &[
    ("GLM", GLM_MODEL_PRESETS),
    ("Kimi", &["kimi-for-coding"]),
    ("Mimo", MIMO_MODEL_PRESETS),
    ("DeepSeek", &["deepseek-v4-pro", "deepseek-v4-flash"]),
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
const MIMO_V2_5_PRO_CONTEXT_WINDOW_TOKENS: u32 = 1_000_000;
const DEFAULT_DEEPSEEK_CONTEXT_WINDOW_TOKENS: u32 = 1_000_000;
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
        .unwrap_or_else(|| "glm-5.3".to_string())
}

fn normalize_glm_model_name(model: &str) -> Option<String> {
    match model.trim().to_ascii_lowercase().as_str() {
        "glm-5.3" => Some("glm-5.3".to_string()),
        _ => None,
    }
}

fn default_mimo_model() -> String {
    std::env::var("MIMO_MODEL")
        .ok()
        .map(|model| normalize_mimo_model_name(&model))
        .unwrap_or_else(|| DEFAULT_MIMO_MODEL.to_string())
}

/// DeepSeek 默认模型：`DEEPSEEK_MODEL` 或 `deepseek-v4-pro`。
pub(crate) fn default_deepseek_model() -> String {
    std::env::var("DEEPSEEK_MODEL")
        .ok()
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
        .unwrap_or_else(|| "deepseek-v4-pro".to_string())
}

fn normalize_mimo_model_name(model: &str) -> String {
    let trimmed = model.trim();
    match trimmed.to_ascii_lowercase().as_str() {
        "mimo-v2.5-pro" | "mimo-v2-5-pro" => DEFAULT_MIMO_MODEL.to_string(),
        "mimo-v2-pro" => "mimo-v2-pro".to_string(),
        "mimo-v2-flash" => "mimo-v2-flash".to_string(),
        "mimo-v2-omni" => "mimo-v2-omni".to_string(),
        _ => trimmed.to_string(),
    }
}

fn default_mimo_context_window_tokens(model: &str) -> u32 {
    match normalize_mimo_model_name(model).as_str() {
        DEFAULT_MIMO_MODEL => MIMO_V2_5_PRO_CONTEXT_WINDOW_TOKENS,
        _ => DEFAULT_MIMO_CONTEXT_WINDOW_TOKENS,
    }
}

// ── Backend selector ──────────────────────────────────────────────────────────

/// 当前使用的 LLM 后端，内部持有已选定的模型名称。
/// 通过 `LLM_PROVIDER=minimax/glm/kimi/mimo/deepseek` 显式指定，
/// 或自动检测：优先顺序为 Kimi > DeepSeek > MiniMax > Mimo > GLM。
#[derive(Clone)]
pub(crate) enum LlmBackend {
    /// GLM 后端，持有当前选定的模型名。
    Glm(String),
    /// Kimi (Moonshot) 后端，持有当前选定的模型名。
    Kimi(String),
    /// Xiaomi MiMo 普通 Chat 后端，持有当前选定的模型名。
    Mimo(String),
    /// DeepSeek 后端，持有当前选定的模型名。
    DeepSeek(String),
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
                let model = default_mimo_model();
                LlmBackend::Mimo(model)
            }
            "deepseek" => {
                let model = default_deepseek_model();
                LlmBackend::DeepSeek(model)
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
                // 自动检测优先级：Kimi > DeepSeek > MiniMax > Mimo > GLM
                if std::env::var("KIMI_API_KEY").is_ok() {
                    let model =
                        std::env::var("KIMI_MODEL").unwrap_or_else(|_| default_kimi_model());
                    LlmBackend::Kimi(model)
                } else if std::env::var("DEEPSEEK_API_KEY").is_ok()
                    && std::env::var("BIGMODEL_API_KEY").is_err()
                {
                    let model = default_deepseek_model();
                    LlmBackend::DeepSeek(model)
                } else if std::env::var("MINIMAX_API_KEY").is_ok()
                    && std::env::var("BIGMODEL_API_KEY").is_err()
                {
                    let model = std::env::var("MINIMAX_MODEL")
                        .unwrap_or_else(|_| "MiniMax-M2.5".to_string());
                    LlmBackend::MiniMax(model)
                } else if std::env::var("MIMO_API_KEY").is_ok()
                    && std::env::var("BIGMODEL_API_KEY").is_err()
                {
                    let model = default_mimo_model();
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
            Self::DeepSeek(_) => "DeepSeek",
            Self::MiniMax(_) => "MiniMax",
        }
    }

    /// 当前选定的模型名。
    pub(crate) fn model_name(&self) -> &str {
        match self {
            Self::Glm(m) | Self::Kimi(m) | Self::Mimo(m) | Self::DeepSeek(m) | Self::MiniMax(m) => {
                m
            }
        }
    }

    pub(crate) fn context_window_tokens(&self) -> u32 {
        env_u32("GOLDBOT_CONTEXT_WINDOW_TOKENS")
            .or_else(|| match self {
                Self::Glm(_) => env_u32("BIGMODEL_CONTEXT_WINDOW_TOKENS")
                    .or_else(|| env_u32("BIGMODEL_CODING_CONTEXT_WINDOW_TOKENS")),
                Self::Kimi(_) => env_u32("KIMI_CONTEXT_WINDOW_TOKENS"),
                Self::Mimo(_) => env_u32("MIMO_CONTEXT_WINDOW_TOKENS"),
                Self::DeepSeek(_) => env_u32("DEEPSEEK_CONTEXT_WINDOW_TOKENS"),
                Self::MiniMax(_) => env_u32("MINIMAX_CONTEXT_WINDOW_TOKENS"),
            })
            .unwrap_or_else(|| match self {
                Self::Glm(_) => DEFAULT_GLM_CONTEXT_WINDOW_TOKENS,
                Self::Kimi(_) => DEFAULT_KIMI_CONTEXT_WINDOW_TOKENS,
                Self::Mimo(model) => default_mimo_context_window_tokens(model),
                Self::DeepSeek(_) => DEFAULT_DEEPSEEK_CONTEXT_WINDOW_TOKENS,
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
        maybe_write_debug_log(messages);
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
            Self::DeepSeek(model) => {
                DeepSeekProvider
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
            Self::Glm(model) => {
                // 标题栏展示当前思考等级，例如 `glm-5.3 · max`。
                let effort = glm_effort_from_env();
                let label = match effort {
                    Some(e) => format!("{model} · {e}"),
                    None => model.clone(),
                };
                (label, base_url_from_env())
            }
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
            Self::DeepSeek(model) => {
                // 标题栏展示当前思考等级，例如 `deepseek-v4-pro · high`。
                let effort = deepseek_effort_from_env();
                let label = match effort {
                    Some(e) => format!("{model} · {e}"),
                    None => model.clone(),
                };
                (
                    label,
                    std::env::var("DEEPSEEK_BASE_URL")
                        .unwrap_or_else(|_| "https://api.deepseek.com".to_string()),
                )
            }
        }
    }

    /// 当前 provider 所需 API Key 的环境变量名。
    pub(crate) fn required_key_name(&self) -> &'static str {
        match self {
            Self::Glm(_) => "BIGMODEL_API_KEY",
            Self::Kimi(_) => "KIMI_API_KEY",
            Self::Mimo(_) => "MIMO_API_KEY",
            Self::DeepSeek(_) => "DEEPSEEK_API_KEY",
            Self::MiniMax(_) => "MINIMAX_API_KEY",
        }
    }
}

fn env_u32(name: &str) -> Option<u32> {
    std::env::var(name).ok()?.trim().parse::<u32>().ok()
}

/// 从 `BIGMODEL_EFFORT` 读取 GLM 思考等级，仅接受 low/high/max，非法值忽略。
pub(crate) fn glm_effort_from_env() -> Option<String> {
    std::env::var("BIGMODEL_EFFORT").ok().and_then(|v| {
        let v = v.trim().to_ascii_lowercase();
        matches!(v.as_str(), "low" | "high" | "max").then_some(v)
    })
}

/// 从 `DEEPSEEK_EFFORT` 读取 DeepSeek 思考强度，仅接受 low/high/max，非法值忽略。
pub(crate) fn deepseek_effort_from_env() -> Option<String> {
    std::env::var("DEEPSEEK_EFFORT").ok().and_then(|v| {
        let v = v.trim().to_ascii_lowercase();
        matches!(v.as_str(), "low" | "high" | "max").then_some(v)
    })
}

/// 全局环境变量测试锁，防止并行测试读写同一环境变量时互相干扰。
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::{
        BACKEND_PRESETS, DEFAULT_MIMO_CONTEXT_WINDOW_TOKENS, DEFAULT_MIMO_MODEL, LlmBackend,
        MIMO_V2_5_PRO_CONTEXT_WINDOW_TOKENS, default_mimo_context_window_tokens,
        normalize_mimo_model_name,
    };

    use super::ENV_LOCK;

    #[test]
    fn glm_display_info_includes_effort_level() {
        let _guard = ENV_LOCK.lock().unwrap();

        unsafe {
            std::env::set_var("BIGMODEL_EFFORT", "high");
        }
        let backend = LlmBackend::Glm("glm-5.3".to_string());
        let (label, _) = backend.display_info();
        assert_eq!(label, "glm-5.3 · high");

        unsafe {
            std::env::remove_var("BIGMODEL_EFFORT");
        }
    }

    #[test]
    fn glm_display_info_without_effort_omits_suffix() {
        let _guard = ENV_LOCK.lock().unwrap();

        unsafe {
            std::env::remove_var("BIGMODEL_EFFORT");
        }
        let backend = LlmBackend::Glm("glm-5.3".to_string());
        let (label, _) = backend.display_info();
        assert_eq!(label, "glm-5.3");
    }

    #[test]
    fn glm_backend_presets_only_include_glm_5_3() {
        let glm_models = BACKEND_PRESETS
            .iter()
            .find(|(label, _)| *label == "GLM")
            .map(|(_, models)| *models)
            .expect("GLM backend preset should exist");

        assert_eq!(glm_models.len(), 1);
        assert!(glm_models.contains(&"glm-5.3"));
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

        assert_eq!(mimo_models.first(), Some(&DEFAULT_MIMO_MODEL));
        assert!(mimo_models.contains(&"mimo-v2.5-pro"));
        assert!(mimo_models.contains(&"mimo-v2-pro"));
        assert!(mimo_models.contains(&"mimo-v2-flash"));
        assert!(mimo_models.contains(&"mimo-v2-omni"));
    }

    #[test]
    fn mimo_model_aliases_normalize_to_api_tag() {
        assert_eq!(normalize_mimo_model_name("MiMo-V2.5-Pro"), "mimo-v2.5-pro");
        assert_eq!(normalize_mimo_model_name("mimo-v2-5-pro"), "mimo-v2.5-pro");
        assert_eq!(normalize_mimo_model_name("mimo-v2-pro"), "mimo-v2-pro");
        assert_eq!(
            normalize_mimo_model_name("custom-mimo-model"),
            "custom-mimo-model"
        );
    }

    #[test]
    fn mimo_context_window_defaults_follow_selected_model() {
        assert_eq!(
            default_mimo_context_window_tokens("mimo-v2.5-pro"),
            MIMO_V2_5_PRO_CONTEXT_WINDOW_TOKENS
        );
        assert_eq!(
            default_mimo_context_window_tokens("mimo-v2-5-pro"),
            MIMO_V2_5_PRO_CONTEXT_WINDOW_TOKENS
        );
        assert_eq!(
            default_mimo_context_window_tokens("mimo-v2-pro"),
            DEFAULT_MIMO_CONTEXT_WINDOW_TOKENS
        );
    }
}
