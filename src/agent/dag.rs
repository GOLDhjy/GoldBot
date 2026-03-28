//! DAG Scheduler for SubAgent execution.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};

use crate::agent::provider::{LlmBackend, Message, Role};
use crate::agent::react::parse_llm_response;
use crate::agent::roles::{BuiltinRole, build_sub_agent_prompt};
use crate::agent::sub_agent::{
    InputMerge, NodeId, OutputMerge, SubAgentIdGen, SubAgentResult, SubAgentStatus, TaskGraph,
    TaskNode,
};
use crate::tools::skills::{Skill, skill_tool_result};

const DEFAULT_SUBAGENT_TIMEOUT_SECS: u64 = 600;
const DEFAULT_SUBAGENT_MAX_STEPS: usize = 30;

/// Configuration for DAG execution
/// 节点完成进度通知
pub struct NodeProgress {
    pub node_id: NodeId,
    pub elapsed: Duration,
    pub success: bool,
}

#[derive(Clone)]
pub struct DagConfig {
    pub http_client: reqwest::Client,
    pub backend: LlmBackend,
    pub base_system_prompt: String,
    pub skills: Arc<Vec<Skill>>,
    pub max_steps: usize,
    pub timeout: Duration,
    pub cancel_flag: Option<Arc<AtomicBool>>,
    pub progress_tx: Option<tokio::sync::mpsc::UnboundedSender<NodeProgress>>,
}

impl DagConfig {
    pub fn new(
        http_client: reqwest::Client,
        backend: LlmBackend,
        base_system_prompt: String,
        skills: Arc<Vec<Skill>>,
    ) -> Self {
        Self {
            http_client,
            backend,
            base_system_prompt,
            skills,
            max_steps: DEFAULT_SUBAGENT_MAX_STEPS,
            timeout: Duration::from_secs(DEFAULT_SUBAGENT_TIMEOUT_SECS),
            cancel_flag: None,
            progress_tx: None,
        }
    }
}

/// Result of DAG execution
pub struct DagResult {
    pub output: String,
    pub node_results: HashMap<NodeId, SubAgentResult>,
    pub elapsed: Duration,
    pub has_failures: bool,
}

fn compute_execution_layers(graph: &TaskGraph) -> Result<Vec<Vec<&TaskNode>>> {
    let node_count = graph.nodes.len();
    if node_count == 0 {
        return Ok(vec![]);
    }

    let mut node_map: HashMap<&NodeId, &TaskNode> = HashMap::with_capacity(node_count);
    let mut in_degree: HashMap<&NodeId, usize> = HashMap::with_capacity(node_count);
    let mut dependents: HashMap<&NodeId, Vec<&NodeId>> = HashMap::with_capacity(node_count);

    for node in &graph.nodes {
        let id = &node.id;
        node_map.insert(id, node);
        in_degree.insert(id, 0);
        dependents.insert(id, vec![]);
    }

    for node in &graph.nodes {
        for dep_id in &node.depends_on {
            if !in_degree.contains_key(dep_id) {
                return Err(anyhow!(
                    "Node \"{}\" depends on unknown node \"{}\"",
                    node.id,
                    dep_id
                ));
            }
            *in_degree.get_mut(&node.id).unwrap() += 1;
            dependents.get_mut(dep_id).unwrap().push(&node.id);
        }
    }

    let mut layers: Vec<Vec<&TaskNode>> = Vec::new();
    let mut queue: VecDeque<&NodeId> = in_degree
        .iter()
        .filter(|(_, deg)| **deg == 0)
        .map(|(&id, _)| id)
        .collect();

    let mut processed = 0;
    while !queue.is_empty() {
        let layer_ids: Vec<&NodeId> = queue.drain(..).collect();
        let layer_nodes: Vec<&TaskNode> = layer_ids
            .iter()
            .filter_map(|id| node_map.get(id).copied())
            .collect();

        for id in &layer_ids {
            for &dependent_id in dependents.get(id).unwrap_or(&vec![]) {
                let deg = in_degree.get_mut(dependent_id).unwrap();
                *deg -= 1;
                if *deg == 0 {
                    queue.push_back(dependent_id);
                }
            }
        }

        processed += layer_nodes.len();
        if !layer_nodes.is_empty() {
            layers.push(layer_nodes);
        }
    }

    if processed != node_count {
        return Err(anyhow!("Cycle detected in TaskGraph"));
    }

    Ok(layers)
}

fn find_leaf_nodes(graph: &TaskGraph) -> Vec<NodeId> {
    let mut has_dependents: HashSet<&NodeId> = HashSet::new();
    for node in &graph.nodes {
        for dep in &node.depends_on {
            has_dependents.insert(dep);
        }
    }
    graph
        .nodes
        .iter()
        .filter(|n| !has_dependents.contains(&n.id))
        .map(|n| n.id.clone())
        .collect()
}

fn merge_inputs(inputs: &[(NodeId, String)], strategy: InputMerge) -> String {
    if inputs.is_empty() {
        return String::new();
    }

    match strategy {
        InputMerge::Concat => inputs
            .iter()
            .map(|(id, output)| format!("[Output from {}]:\n{}", id, output))
            .collect::<Vec<_>>()
            .join("\n\n---\n\n"),
        InputMerge::Structured => {
            let entries: Vec<serde_json::Value> = inputs
                .iter()
                .map(|(id, output)| serde_json::json!({"from": id, "output": output}))
                .collect();
            serde_json::to_string_pretty(&entries).unwrap_or_else(|_| "[]".to_string())
        }
    }
}

fn merge_outputs(results: &[(NodeId, String)], strategy: OutputMerge) -> String {
    if results.is_empty() {
        return String::new();
    }

    match strategy {
        OutputMerge::All => {
            let entries: Vec<serde_json::Value> = results
                .iter()
                .map(|(id, output)| serde_json::json!({"node": id, "output": output}))
                .collect();
            serde_json::to_string_pretty(&entries).unwrap_or_else(|_| "[]".to_string())
        }
        OutputMerge::First => results.first().map(|(_, o)| o.clone()).unwrap_or_default(),
        OutputMerge::Concat => results
            .iter()
            .map(|(id, output)| format!("[{}]:\n{}", id, output))
            .collect::<Vec<_>>()
            .join("\n\n"),
    }
}

async fn run_subagent_worker(
    node: TaskNode,
    merged_input: Option<String>,
    config: DagConfig,
    id_gen: Arc<SubAgentIdGen>,
    cancel_flag: Option<Arc<AtomicBool>>,
) -> SubAgentResult {
    let id = id_gen.next();
    let node_id = node.id.clone();
    let started_at = Instant::now();

    if cancel_flag
        .as_ref()
        .map(|f| f.load(Ordering::SeqCst))
        .unwrap_or(false)
    {
        return SubAgentResult {
            id,
            node_id,
            status: SubAgentStatus::Cancelled,
            output: String::new(),
            messages: vec![],
            elapsed: started_at.elapsed(),
        };
    }

    let role = node.role.as_ref().and_then(|r| BuiltinRole::from_str(r));
    let system_prompt = build_sub_agent_prompt(
        node.system_prompt.as_deref(),
        role.as_ref(),
        &config.base_system_prompt,
    );
    let mut messages: Vec<Message> = vec![Message::system(&system_prompt)];
    let user_content = match merged_input {
        Some(input) => format!("Task: {}\n\n---\n\nUpstream Inputs:\n{}", node.task, input),
        None => node.task.clone(),
    };
    messages.push(Message::user(&user_content));

    let backend = node
        .model
        .as_ref()
        .map(|m| match &config.backend {
            LlmBackend::Glm(_) => LlmBackend::Glm(m.clone()),
            LlmBackend::Kimi(_) => LlmBackend::Kimi(m.clone()),
            LlmBackend::MiniMax(_) => LlmBackend::MiniMax(m.clone()),
        })
        .unwrap_or_else(|| config.backend.clone());

    let mut output = String::new();
    let mut status = SubAgentStatus::Completed;

    'react_loop: for _step in 0..config.max_steps {
        if cancel_flag
            .as_ref()
            .map(|f| f.load(Ordering::SeqCst))
            .unwrap_or(false)
        {
            status = SubAgentStatus::Cancelled;
            break;
        }
        if started_at.elapsed() > config.timeout {
            status = SubAgentStatus::Timeout;
            break;
        }

        let llm_result = backend
            .chat_stream_with(&config.http_client, &messages, false, |_| {}, |_| {})
            .await;

        match llm_result {
            Ok((response, _usage)) => {
                messages.push(Message::assistant(&response));
                match parse_llm_response(&response) {
                    Ok((_text, actions)) => {
                        let mut found_final = false;
                        for action in actions {
                            let tool_result = match action {
                                crate::types::LlmAction::Final { summary } => {
                                    output = summary;
                                    found_final = true;
                                    break;
                                }
                                crate::types::LlmAction::Shell { command } => {
                                    tokio::task::spawn_blocking(move || {
                                        execute_shell_command(command)
                                    })
                                    .await
                                    .unwrap_or_else(|_| "[Shell: task panicked]".to_string())
                                }
                                crate::types::LlmAction::ReadFile {
                                    path,
                                    offset,
                                    limit,
                                } => tokio::task::spawn_blocking(move || {
                                    execute_read(&path, offset, limit)
                                })
                                .await
                                .unwrap_or_else(|_| "[Read: task panicked]".to_string()),
                                crate::types::LlmAction::WriteFile { path, content } => {
                                    tokio::task::spawn_blocking(move || {
                                        execute_write(&path, &content)
                                    })
                                    .await
                                    .unwrap_or_else(|_| "[Write: task panicked]".to_string())
                                }
                                crate::types::LlmAction::UpdateFile {
                                    path,
                                    line_start,
                                    line_end,
                                    new_string,
                                } => tokio::task::spawn_blocking(move || {
                                    execute_update(&path, line_start, line_end, &new_string)
                                })
                                .await
                                .unwrap_or_else(|_| "[Update: task panicked]".to_string()),
                                crate::types::LlmAction::SearchFiles { pattern, path } => {
                                    tokio::task::spawn_blocking(move || {
                                        execute_search(&pattern, &path)
                                    })
                                    .await
                                    .unwrap_or_else(|_| "[Search: task panicked]".to_string())
                                }
                                crate::types::LlmAction::GlobFiles { pattern, path } => {
                                    tokio::task::spawn_blocking(move || {
                                        execute_glob(&pattern, &path)
                                    })
                                    .await
                                    .unwrap_or_else(|_| "[Glob: task panicked]".to_string())
                                }
                                crate::types::LlmAction::WebSearch { query } => {
                                    tokio::task::spawn_blocking(move || execute_web_search(&query))
                                        .await
                                        .unwrap_or_else(|_| {
                                            "[WebSearch: task panicked]".to_string()
                                        })
                                }
                                crate::types::LlmAction::Skill { name } => {
                                    skill_tool_result(config.skills.as_ref(), &name)
                                }
                                _ => "[Action not supported in SubAgent]".to_string(),
                            };
                            messages.push(Message::user(&tool_result));
                        }
                        if found_final {
                            break 'react_loop;
                        }
                    }
                    Err(e) => messages.push(Message::user(format!("[Parse error: {e}]"))),
                }
            }
            Err(e) => {
                status = SubAgentStatus::Failed(format!("LLM error: {e}"));
                break;
            }
        }
    }

    if matches!(status, SubAgentStatus::Completed) && output.is_empty() {
        output = messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, Role::Assistant))
            .map(|m| m.content.clone())
            .unwrap_or_else(|| "[No output]".to_string());
    }

    let elapsed = started_at.elapsed();
    if let Some(tx) = &config.progress_tx {
        let success = matches!(status, SubAgentStatus::Completed);
        let _ = tx.send(NodeProgress {
            node_id: node_id.clone(),
            elapsed,
            success,
        });
    }
    SubAgentResult {
        id,
        node_id,
        status,
        output,
        messages,
        elapsed,
    }
}

fn execute_shell_command(command: String) -> String {
    use std::process::Command;
    #[cfg(target_os = "windows")]
    let output = Command::new("powershell")
        .args(["-Command", &command])
        .output();
    #[cfg(not(target_os = "windows"))]
    let output = Command::new("bash").args(["-c", &command]).output();
    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            if out.status.success() {
                format!("[Shell exit=0]\n{stdout}")
            } else {
                format!(
                    "[Shell exit={}]\n{stdout}\n{stderr}",
                    out.status.code().unwrap_or(-1)
                )
            }
        }
        Err(e) => format!("[Shell error: {e}]"),
    }
}

fn execute_read(path: &str, offset: Option<usize>, limit: Option<usize>) -> String {
    match std::fs::read_to_string(path) {
        Ok(content) => {
            let lines: Vec<&str> = content.lines().collect();
            let total = lines.len();
            let start = offset.map(|o| o.saturating_sub(1)).unwrap_or(0).min(total);
            let end = limit.map(|l| (start + l).min(total)).unwrap_or(total);
            let num_width = total.to_string().len().max(1);
            let body: String = lines[start..end]
                .iter()
                .enumerate()
                .map(|(i, l)| format!("{:>num_width$}: {l}", start + i + 1))
                .collect::<Vec<_>>()
                .join("\n");
            if end < total {
                format!("{body}\n... ({} more lines)", total - end)
            } else {
                body
            }
        }
        Err(e) => format!("[Read error: {e}]"),
    }
}

fn execute_write(path: &str, content: &str) -> String {
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(path, content) {
        Ok(()) => format!("[File written: {path}]"),
        Err(e) => format!("[Write error: {e}]"),
    }
}

fn execute_search(pattern: &str, path: &str) -> String {
    let path = Some(path);
    use regex::Regex;
    use std::fs;
    use std::path::Path;

    let search_path = path.unwrap_or(".");
    let pattern_regex =
        Regex::new(pattern).unwrap_or_else(|_| Regex::new(&regex::escape(pattern)).unwrap());

    let mut results = Vec::new();
    let mut file_count = 0;
    const MAX_FILES: usize = 50;
    const MAX_MATCHES: usize = 100;

    fn search_dir(dir: &Path, pattern: &Regex, results: &mut Vec<String>, file_count: &mut usize) {
        if *file_count >= MAX_FILES || results.len() >= MAX_MATCHES {
            return;
        }
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.starts_with('.') || n == "target" || n == "node_modules")
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    search_dir(&path, pattern, results, file_count);
                } else if path.is_file() {
                    *file_count += 1;
                    if *file_count > MAX_FILES {
                        return;
                    }
                    if let Ok(content) = fs::read_to_string(&path) {
                        for (line_num, line) in content.lines().enumerate() {
                            if pattern.is_match(line) {
                                results.push(format!(
                                    "{}:{}: {}",
                                    path.display(),
                                    line_num + 1,
                                    line.trim()
                                ));
                                if results.len() >= MAX_MATCHES {
                                    return;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    search_dir(
        Path::new(search_path),
        &pattern_regex,
        &mut results,
        &mut file_count,
    );

    if results.is_empty() {
        format!("[No matches for: {}]", pattern)
    } else {
        format!("[Search: {}]\n{}", pattern, results.join("\n"))
    }
}

fn execute_glob(pattern: &str, path: &str) -> String {
    match crate::tools::glob::glob_files(pattern, path) {
        Ok(result) => result.output,
        Err(e) => format!("[Glob error: {e}]"),
    }
}

fn execute_update(path: &str, line_start: usize, line_end: usize, new_string: &str) -> String {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => return format!("[Update error: {}]", e),
    };
    let crlf = content.contains("\r\n");
    let normalized = content.replace("\r\n", "\n");
    let mut lines: Vec<&str> = normalized.lines().collect();
    let total = lines.len();
    if line_start == 0 || line_start > total + 1 || line_end < line_start || line_end > total {
        return format!(
            "[Update error: invalid range {line_start}-{line_end}, file has {total} lines]"
        );
    }
    let s = line_start - 1;
    let e = line_end;
    let norm_new = new_string.replace("\r\n", "\n");
    let new_lines: Vec<&str> = norm_new.lines().collect();
    lines.splice(s..e, new_lines.iter().copied());
    let mut result = lines.join("\n");
    if normalized.ends_with('\n') {
        result.push('\n');
    }
    let final_content = if crlf {
        result.replace("\n", "\r\n")
    } else {
        result
    };
    match std::fs::write(path, final_content) {
        Ok(()) => format!("[Updated {path}: lines {line_start}-{line_end} replaced]"),
        Err(e) => format!("[Update write error: {}]", e),
    }
}

fn execute_web_search(query: &str) -> String {
    match crate::tools::web_search::search(query) {
        Ok(result) => result.output,
        Err(e) => format!("[Web search error: {e}]"),
    }
}

/// 构建 DAG 树形字符串（可带节点完成标记）。
///
/// `done`: node_id -> (success, elapsed_secs)，已完成节点。
pub fn build_dag_tree(
    nodes: &[TaskNode],
    output_nodes: &[NodeId],
    done: &HashMap<NodeId, (bool, f64)>,
) -> String {
    // 计算每个节点的层级（拓扑排序）
    let mut depth: HashMap<&str, usize> = HashMap::new();
    let mut changed = true;
    while changed {
        changed = false;
        for n in nodes {
            let d = n
                .depends_on
                .iter()
                .map(|dep| depth.get(dep.as_str()).copied().unwrap_or(0) + 1)
                .max()
                .unwrap_or(0);
            let entry = depth.entry(n.id.as_str()).or_insert(0);
            if *entry < d {
                *entry = d;
                changed = true;
            }
        }
    }
    let max_depth = depth.values().copied().max().unwrap_or(0);
    let output_set: HashSet<&str> = output_nodes.iter().map(|s| s.as_str()).collect();
    let mut lines = vec![format!("SubAgent DAG  ({} nodes)", nodes.len())];
    for layer in 0..=max_depth {
        let mut layer_nodes: Vec<&TaskNode> = nodes
            .iter()
            .filter(|n| depth.get(n.id.as_str()).copied().unwrap_or(0) == layer)
            .collect();
        layer_nodes.sort_by(|a, b| a.id.cmp(&b.id));
        let count = layer_nodes.len();
        for (i, n) in layer_nodes.iter().enumerate() {
            let branch = if i + 1 == count { "└─" } else { "├─" };
            let role_hint = n
                .role
                .as_deref()
                .map(|r| format!(" [{r}]"))
                .unwrap_or_default();
            let deps = if n.depends_on.is_empty() {
                String::new()
            } else {
                format!("  ← {}", n.depends_on.join(", "))
            };
            let output_mark = if output_set.contains(n.id.as_str())
                || (output_set.is_empty()
                    && !nodes.iter().any(|other| other.depends_on.contains(&n.id)))
            {
                " ★"
            } else {
                ""
            };
            // 完成标记
            let done_mark = if let Some((ok, secs)) = done.get(&n.id) {
                let sym = if *ok { "✓" } else { "✗" };
                format!(" {sym}({:.1}s)", secs)
            } else {
                String::new()
            };
            lines.push(format!(
                "  {branch} {}{role_hint}{deps}{output_mark}{done_mark}",
                n.id
            ));
        }
    }
    lines.join("\n")
}

/// Execute a TaskGraph and return merged results (async, non-blocking)
pub async fn execute(graph: TaskGraph, config: DagConfig) -> Result<DagResult> {
    let start_time = Instant::now();
    let id_gen = Arc::new(SubAgentIdGen::new());
    let results: Arc<Mutex<HashMap<NodeId, SubAgentResult>>> = Arc::new(Mutex::new(HashMap::new()));

    let layers = compute_execution_layers(&graph).context("Failed to compute layers")?;

    let output_node_ids: Vec<NodeId> = if graph.output_nodes.is_empty() {
        find_leaf_nodes(&graph)
    } else {
        graph.output_nodes.clone()
    };

    for layer in &layers {
        if config
            .cancel_flag
            .as_ref()
            .map(|f| f.load(Ordering::SeqCst))
            .unwrap_or(false)
        {
            break;
        }

        let mut join_set = tokio::task::JoinSet::new();

        for node in layer {
            let merged_input = if node.depends_on.is_empty() {
                None
            } else {
                let guard = results.lock().unwrap();
                let inputs: Vec<(NodeId, String)> = node
                    .depends_on
                    .iter()
                    .filter_map(|dep| guard.get(dep).map(|r| (dep.clone(), r.output.clone())))
                    .collect();
                if inputs.len() != node.depends_on.len() {
                    None
                } else {
                    Some(merge_inputs(&inputs, node.input_merge))
                }
            };

            let node = (*node).clone();
            let config = config.clone();
            let id_gen = Arc::clone(&id_gen);
            let cancel_flag = config.cancel_flag.clone();

            join_set.spawn(async move {
                run_subagent_worker(node, merged_input, config, id_gen, cancel_flag).await
            });
        }

        while let Some(join_result) = join_set.join_next().await {
            if let Ok(result) = join_result {
                results
                    .lock()
                    .unwrap()
                    .insert(result.node_id.clone(), result);
            }
        }
    }

    let results_guard = results.lock().unwrap();
    let has_failures = results_guard
        .values()
        .any(|r| !matches!(r.status, SubAgentStatus::Completed));
    let outputs: Vec<(NodeId, String)> = output_node_ids
        .iter()
        .filter_map(|id| {
            results_guard
                .get(id)
                .map(|r| (id.clone(), r.output.clone()))
        })
        .collect();

    Ok(DagResult {
        output: merge_outputs(&outputs, graph.output_merge),
        node_results: results_guard.clone(),
        elapsed: start_time.elapsed(),
        has_failures,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_node(id: &str, task: &str, depends_on: Vec<&str>) -> TaskNode {
        TaskNode {
            id: id.to_string(),
            task: task.to_string(),
            model: None,
            role: None,
            system_prompt: None,
            depends_on: depends_on.iter().map(|s| s.to_string()).collect(),
            input_merge: InputMerge::default(),
        }
    }

    #[test]
    fn test_topological_sort() {
        let graph = TaskGraph {
            nodes: vec![
                make_node("a", "t1", vec![]),
                make_node("b", "t2", vec!["a"]),
            ],
            output_nodes: vec!["b".to_string()],
            output_merge: OutputMerge::default(),
        };
        let layers = compute_execution_layers(&graph).unwrap();
        assert_eq!(layers.len(), 2);
    }

    #[test]
    fn test_cycle_detection() {
        let graph = TaskGraph {
            nodes: vec![
                make_node("a", "t1", vec!["b"]),
                make_node("b", "t2", vec!["a"]),
            ],
            output_nodes: vec![],
            output_merge: OutputMerge::default(),
        };
        assert!(compute_execution_layers(&graph).is_err());
    }

    #[test]
    fn test_merge_outputs() {
        let outputs = vec![("a".into(), "out a".into()), ("b".into(), "out b".into())];
        let merged = merge_outputs(&outputs, OutputMerge::First);
        assert_eq!(merged, "out a");
    }
}
