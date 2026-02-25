use super::{
    config::{RawServerEntry, extract_local_command_and_args, parse_server_entries},
    types::McpToolSpec,
    util::{
        normalize_action_name_for_lookup, normalize_arguments_for_tool, sanitize_token,
        summarize_input_schema, unique_action_name,
    },
};
use serde_json::json;
use std::collections::BTreeSet;

#[test]
fn sanitize_token_collapses_symbols() {
    assert_eq!(sanitize_token("Context7 MCP"), "context7_mcp");
    assert_eq!(sanitize_token("@@"), "tool");
    assert_eq!(sanitize_token("A__B"), "a_b");
}

#[test]
fn unique_action_name_adds_suffix() {
    let mut used = BTreeSet::new();
    let first = unique_action_name("context7", "lookup", &mut used);
    let second = unique_action_name("context7", "lookup", &mut used);
    assert_eq!(first, "mcp_context7_lookup");
    assert_eq!(second, "mcp_context7_lookup_2");
}

#[test]
fn summarize_schema_marks_required_fields() {
    let schema = json!({
        "type": "object",
        "properties": {
            "libraryName": { "type": "string" },
            "tokens": { "type": "integer" }
        },
        "required": ["libraryName"]
    });
    let summary = summarize_input_schema(&schema);
    assert!(summary.contains("libraryName:string*"));
    assert!(summary.contains("tokens:integer"));
}

#[test]
fn parse_server_entries_accepts_mcp_wrapper() {
    let raw = r#"{
        "mcp": {
            "context7": { "type": "local", "command": "npx", "args": ["-y", "@upstash/context7-mcp"] }
        }
    }"#;
    let parsed = parse_server_entries(raw).expect("parse should succeed");
    assert!(matches!(
        parsed.get("context7"),
        Some(RawServerEntry::Config(_))
    ));
}

#[test]
fn command_array_is_supported() {
    let raw = r#"{
        "context7": {
            "type": "local",
            "command": ["npx", "-y", "@upstash/context7-mcp", "--api-key", "k"]
        }
    }"#;
    let parsed = parse_server_entries(raw).expect("parse should succeed");
    let RawServerEntry::Config(cfg) = parsed.get("context7").expect("missing config") else {
        panic!("expected config entry");
    };
    let (cmd, args) = extract_local_command_and_args(cfg).expect("command should parse");
    assert_eq!(cmd, "npx");
    assert_eq!(args, vec!["-y", "@upstash/context7-mcp", "--api-key", "k"]);
}

#[test]
fn normalize_action_name_handles_double_underscore() {
    assert_eq!(
        normalize_action_name_for_lookup("mcp__context7__get_repository").as_deref(),
        Some("mcp_context7_get_repository")
    );
}

#[test]
fn context7_resolve_library_id_autofills_query() {
    let spec = McpToolSpec {
        action_name: "mcp_context7_resolve_library_id".to_string(),
        server_name: "context7".to_string(),
        tool_name: "resolve-library-id".to_string(),
        description: String::new(),
        read_only_hint: true,
        input_schema: json!({}),
    };
    let args = json!({ "libraryName": "tokio" });
    let normalized = normalize_arguments_for_tool(&spec, &args);
    assert_eq!(normalized, json!({"libraryName":"tokio","query":"tokio"}));
}
