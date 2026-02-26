#![allow(dead_code)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::agent::provider::{LlmBackend, Message};

// ── SubAgent 标识 ─────────────────────────────────────────────
/// 每个 SubAgent 实例的唯一运行时 ID（调度器内部分配）
pub type SubAgentId = u64;

/// DAG 节点 ID — 字符串，由 LLM 在 graph JSON 中指定，便于引用依赖
pub type NodeId = String;

// ── DAG 节点 ─────────────────────────────────────────────────
/// DAG 中的一个任务节点
#[derive(Debug, Clone)]
pub struct TaskNode {
    /// 节点唯一标识，在同一 TaskGraph 中不可重复
    pub id: NodeId,
    /// 子任务描述，直接传给 Sub-Agent 作为其任务（应包含所有必要上下文）
    pub task: String,
    /// 可选指定后端模型；None = 继承主 Agent 当前模型
    pub model: Option<String>,
    /// 前缀提示词，二选一填法：
    /// - `role`：内置预设名（"search" / "coding" / "analysis" / "writer" / "reviewer"）
    /// - `system_prompt`：自定义文本（优先级高于 role）
    /// 两者都是在默认执行提示词前面加一段前缀，最终结构：[前缀] + [默认执行提示词]
    pub role: Option<String>,
    pub system_prompt: Option<String>,
    /// 上游依赖节点 ID 列表；空 = 无依赖，可立即启动
    pub depends_on: Vec<NodeId>,
    /// 当有多个上游依赖时，如何合并它们的输出作为本节点的输入
    pub input_merge: InputMerge,
}

/// 多上游输出的合并策略
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputMerge {
    /// 按上游节点完成顺序顺序拼接输出文本（默认）
    #[default]
    Concat,
    /// 以结构化 JSON 数组传入：`[{"from":"a","output":"..."},...]`
    /// 适合下游节点需要区分各路输入来源的场景
    Structured,
}

impl InputMerge {
    pub fn from_str(s: &str) -> Self {
        match s.trim() {
            "structured" => Self::Structured,
            _ => Self::Concat,
        }
    }
}

// ── 完整 DAG ─────────────────────────────────────────────────
/// 主 Agent 一次性提交的完整任务 DAG
///
/// 示例（A、B 并行 → C，C 和 D 的结果合并返回）:
/// ```json
/// {
///   "nodes": [
///     {"id": "a", "task": "..."},
///     {"id": "b", "task": "..."},
///     {"id": "c", "task": "...", "depends_on": ["a", "b"]},
///     {"id": "d", "task": "..."}
///   ],
///   "output_nodes": ["c", "d"],
///   "output_merge": "all"
/// }
/// ```
/// 调度器根据 `depends_on` 拓扑排序自动推导并行/串行，无需手动指定。
#[derive(Debug, Clone)]
pub struct TaskGraph {
    pub nodes: Vec<TaskNode>,
    /// 哪些节点的输出最终汇总返回给主 Agent；空 = 所有叶节点
    pub output_nodes: Vec<NodeId>,
    /// 最终多路输出的汇总策略
    pub output_merge: OutputMerge,
}

/// 最终输出汇总策略
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputMerge {
    /// 收集所有 output_nodes 的完整结果（默认）
    #[default]
    All,
    /// 只取最先完成的节点输出（竞争/赛马模式）
    First,
    /// 将所有输出文本顺序拼接后作为单一结果
    Concat,
}

impl OutputMerge {
    pub fn from_str(s: &str) -> Self {
        match s.trim() {
            "first" => Self::First,
            "concat" => Self::Concat,
            _ => Self::All,
        }
    }
}

// ── SubAgent 配置 ─────────────────────────────────────────────
/// 调度器实例化 Sub-Agent 时使用的配置
pub struct SubAgentConfig {
    pub backend: LlmBackend,
    pub system_prompt: Option<String>,
    pub max_steps: usize,
    pub timeout: Duration,
}

// ── 请求/响应消息 ────────────────────────────────────────────
/// 调度器向单个 Sub-Agent 发送的执行请求
pub struct SubAgentRequest {
    pub id: SubAgentId,
    /// 对应的 DAG 节点 ID
    pub node_id: NodeId,
    pub task: String,
    pub config: SubAgentConfig,
    /// 上游节点的输出（已按 InputMerge 策略合并）
    pub input: Option<String>,
    /// 可选的历史上下文消息
    pub context: Vec<Message>,
}

/// Sub-Agent 完成后返回给调度器的结果
pub struct SubAgentResult {
    pub id: SubAgentId,
    pub node_id: NodeId,
    pub status: SubAgentStatus,
    pub output: String,
    pub messages: Vec<Message>,
    pub elapsed: Duration,
}

/// Sub-Agent 任务的完成状态
pub enum SubAgentStatus {
    Completed,
    Failed(String),
    Timeout,
    Cancelled,
}

// ── 运行时状态 ──────────────────────────────────────────────
/// 调度器对单个 Sub-Agent 实例的运行时追踪
pub struct SubAgentHandle {
    pub id: SubAgentId,
    pub node_id: NodeId,
    pub config: SubAgentConfig,
    pub status: SubAgentRunState,
}

pub enum SubAgentRunState {
    /// 依赖尚未全部满足，等待中
    Pending,
    /// 已启动
    Running { started_at: Instant },
    /// 已完成（含成功/失败/超时）
    Done(SubAgentResult),
}

// ── ID 生成器 ───────────────────────────────────────────────
pub struct SubAgentIdGen(AtomicU64);

impl SubAgentIdGen {
    pub fn new() -> Self {
        Self(AtomicU64::new(1))
    }

    pub fn next(&self) -> SubAgentId {
        self.0.fetch_add(1, Ordering::Relaxed)
    }
}

impl Default for SubAgentIdGen {
    fn default() -> Self {
        Self::new()
    }
}
