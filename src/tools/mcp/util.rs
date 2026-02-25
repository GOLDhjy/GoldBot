use std::collections::BTreeSet;

use serde_json::Value;

use super::{MAX_FIELD_CHARS, MAX_SCHEMA_FIELDS, types::McpToolSpec};

pub(super) fn unique_action_name(
    server_name: &str,
    tool_name: &str,
    used: &mut BTreeSet<String>,
) -> String {
    let server = sanitize_token(server_name);
    let tool = sanitize_token(tool_name);
    let base = format!("mcp_{server}_{tool}");

    if used.insert(base.clone()) {
        return base;
    }

    let mut index = 2usize;
    loop {
        let candidate = format!("{base}_{index}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        index += 1;
    }
}

pub(super) fn sanitize_token(input: &str) -> String {
    let mut out = String::new();
    let mut prev_underscore = false;

    for ch in input.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_underscore = false;
        } else if !prev_underscore {
            out.push('_');
            prev_underscore = true;
        }
    }

    let trimmed = out.trim_matches('_').to_string();
    if trimmed.is_empty() {
        "tool".to_string()
    } else {
        trimmed
    }
}

pub(super) fn normalize_action_name_for_lookup(action_name: &str) -> Option<String> {
    let trimmed = action_name.trim();
    if trimmed.is_empty() {
        return None;
    }

    let normalized = sanitize_token(trimmed);
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

pub(super) fn summarize_input_schema(schema: &Value) -> String {
    let required: BTreeSet<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return "object".to_string();
    };
    if properties.is_empty() {
        return "object".to_string();
    }

    let mut keys: Vec<&str> = properties.keys().map(String::as_str).collect();
    keys.sort_unstable();

    let mut parts = Vec::new();
    for key in keys.iter().take(MAX_SCHEMA_FIELDS) {
        let ty = properties
            .get(*key)
            .and_then(|v| v.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("any");
        let req = if required.contains(*key) { "*" } else { "" };
        parts.push(format!(
            "{}:{}{}",
            truncate_chars(key, MAX_FIELD_CHARS),
            truncate_chars(ty, MAX_FIELD_CHARS),
            req
        ));
    }

    if keys.len() > MAX_SCHEMA_FIELDS {
        parts.push(format!("+{}", keys.len() - MAX_SCHEMA_FIELDS));
    }

    parts.join(", ")
}

pub(super) fn fallback_if_empty<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.trim().is_empty() {
        fallback
    } else {
        value
    }
}

pub(super) fn truncate_chars(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub(super) fn normalize_arguments_for_tool(tool: &McpToolSpec, arguments: &Value) -> Value {
    let Some(obj) = arguments.as_object() else {
        return arguments.clone();
    };

    let mut normalized = obj.clone();

    // Context7 `resolve-library-id` requires both `libraryName` and `query`.
    // When the model only provides `libraryName`, mirror it into `query`
    // to avoid a noisy first-call validation failure.
    if tool.server_name == "context7"
        && tool.tool_name == "resolve-library-id"
        && !normalized.contains_key("query")
        && let Some(library_name) = normalized
            .get("libraryName")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    {
        normalized.insert("query".to_string(), Value::String(library_name.to_string()));
    }

    Value::Object(normalized)
}

/// Replace `${VAR_NAME}` placeholders in a string with environment variable values.
pub(super) fn resolve_env_var_refs(s: &str) -> String {
    let mut result = s.to_string();
    loop {
        let Some(start) = result.find("${") else {
            break;
        };
        let Some(rel_end) = result[start + 2..].find('}') else {
            break;
        };
        let var_name = result[start + 2..start + 2 + rel_end].to_string();
        let value = std::env::var(&var_name).unwrap_or_default();
        result = format!(
            "{}{}{}",
            &result[..start],
            value,
            &result[start + 2 + rel_end + 1..]
        );
    }
    result
}
