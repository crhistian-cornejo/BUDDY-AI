// Ported from MIKA (MIT, revision d050bc5): apps/macos/Sources/Agents/ClaudeCode/PermissionRequestFormatter.swift
//! Turns a PermissionRequest payload (Claude Code or Codex) into what the approval card shows: a human title for
//! the tool, a one-line summary, and the full detail — the whole shell command, then every other argument, so
//! nothing the user approves stays hidden.

use serde_json::Value;

/// What the notch shows for one permission request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalText {
    pub title: String,
    pub summary: String,
    pub detail: String,
    /// False when the relay had to cut the payload: "Permitir" is not offered for something not shown whole.
    pub can_allow: bool,
}

/// Primary arguments, shown as "Etiqueta: valor" lines above the command block.
const PRIMARY_KEYS: &[(&str, &str)] = &[
    ("file_path", "Archivo"),
    ("notebook_path", "Cuaderno"),
    ("path", "Ruta"),
    ("url", "URL"),
    ("pattern", "Patrón"),
    ("glob", "Filtro"),
    ("query", "Búsqueda"),
    ("description", "Descripción"),
    ("subagent_type", "Agente"),
];

/// Longest summary line, in characters.
const SUMMARY_LEN: usize = 90;

pub fn approval_text(payload: &Value) -> ApprovalText {
    let tool = payload.get("tool_name").and_then(Value::as_str).unwrap_or("").trim();
    let empty = serde_json::Map::new();
    let input = payload.get("tool_input").and_then(Value::as_object).unwrap_or(&empty);
    let text = |key: &str| input.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());

    let mut lines: Vec<String> = Vec::new();
    let mut shown: Vec<&str> = Vec::new();

    // MCP tools are named "mcp__<server>__<tool>".
    let mcp = tool.strip_prefix("mcp__").map(|rest| {
        let mut parts = rest.splitn(2, "__");
        (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string())
    });
    if let Some((server, name)) = &mcp {
        if !server.is_empty() {
            lines.push(format!("Servidor: {server}"));
        }
        if !name.is_empty() {
            lines.push(format!("Herramienta: {name}"));
        }
    }

    for (key, label) in PRIMARY_KEYS {
        if let Some(value) = text(key) {
            lines.push(format!("{label}: {value}"));
            shown.push(key);
        }
    }

    // Main block: the full shell command when there is one, then every other argument.
    let mut sections: Vec<String> = Vec::new();
    let command = command_of(input);
    if let Some(command) = &command {
        sections.push(command.clone());
        shown.push("command");
    }
    let mut rest: Vec<(&String, &Value)> = input.iter().filter(|(k, _)| !shown.contains(&k.as_str())).collect();
    rest.sort_by(|a, b| a.0.cmp(b.0));
    for (key, value) in rest {
        match value {
            Value::String(s) => sections.push(format!("{key}:\n{s}")),
            other => sections.push(format!("{key}: {}", serde_json::to_string_pretty(other).unwrap_or_default())),
        }
    }

    let can_allow = payload.get("_truncated").and_then(Value::as_bool) != Some(true);
    let mut detail = lines.join("\n");
    if !sections.is_empty() {
        if !detail.is_empty() {
            detail.push_str("\n\n");
        }
        detail.push_str(&sections.join("\n\n"));
    }
    if !can_allow {
        if !detail.is_empty() {
            detail.push_str("\n\n");
        }
        detail.push_str("La petición llegó recortada: respóndela en la terminal para verla entera.");
    }

    ApprovalText {
        title: title_for(tool, mcp.as_ref().map(|(s, n)| (s.as_str(), n.as_str()))),
        summary: summary_for(tool, input, command.as_deref()),
        detail,
        can_allow,
    }
}

/// The shell command, whatever the agent calls it: a string (`command`, Claude Code's Bash and Codex's shell tool),
/// or an argv array (`command: ["bash", "-lc", "…"]`, some Codex tools).
fn command_of(input: &serde_json::Map<String, Value>) -> Option<String> {
    match input.get("command")? {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => {
            let words: Vec<&str> = parts.iter().filter_map(Value::as_str).collect();
            (!words.is_empty()).then(|| words.join(" "))
        }
        _ => None,
    }
}

/// "Ejecutar un comando", "Editar un archivo"… — what the agent wants to do, in plain words.
fn title_for(tool: &str, mcp: Option<(&str, &str)>) -> String {
    if let Some((server, name)) = mcp {
        return match (name.is_empty(), server.is_empty()) {
            (false, false) => format!("Usar «{name}» de {server}"),
            (false, true) => format!("Usar «{name}»"),
            _ => "Usar una herramienta MCP".into(),
        };
    }
    let title = match tool {
        "Bash" | "shell" | "exec_command" | "local_shell" | "unified_exec" => "Ejecutar un comando",
        "Write" => "Escribir un archivo",
        "Edit" | "MultiEdit" => "Editar un archivo",
        "apply_patch" => "Aplicar cambios a archivos",
        "Read" => "Leer un archivo",
        "NotebookEdit" => "Editar un cuaderno",
        "WebFetch" => "Abrir una página web",
        "WebSearch" | "web_search" => "Buscar en la web",
        "Glob" => "Buscar archivos",
        "Grep" => "Buscar dentro de archivos",
        "LS" => "Ver una carpeta",
        "Task" | "Agent" => "Lanzar un subagente",
        "TodoWrite" => "Actualizar la lista de tareas",
        "ExitPlanMode" => "Empezar con el plan",
        "" => "Usar una herramienta",
        other => return format!("Usar {other}"),
    };
    title.into()
}

/// One line saying exactly what: the command's first line, the file's name, the site, the search.
fn summary_for(tool: &str, input: &serde_json::Map<String, Value>, command: Option<&str>) -> String {
    let text = |key: &str| input.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let line = if let Some(command) = command {
        command.trim().to_string()
    } else if let Some(path) = text("file_path").or_else(|| text("notebook_path")).or_else(|| text("path")) {
        file_name(path).to_string()
    } else if let Some(url) = text("url") {
        host(url).unwrap_or(url).to_string()
    } else if let Some(q) = text("query").or_else(|| text("pattern")).or_else(|| text("glob")) {
        q.to_string()
    } else if let Some(d) = text("description").or_else(|| text("prompt")) {
        d.to_string()
    } else if tool.starts_with("mcp__") || tool.is_empty() {
        String::new()
    } else {
        tool.to_string()
    };
    one_line(&line, SUMMARY_LEN)
}

/// The first non-empty line, shortened to `limit` characters with "…".
fn one_line(text: &str, limit: usize) -> String {
    let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let more_lines = text.trim().lines().count() > 1;
    let mut out: String = first.chars().take(limit).collect();
    if first.chars().count() > limit || more_lines {
        out.push('…');
    }
    out
}

fn file_name(path: &str) -> &str {
    path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().filter(|s| !s.is_empty()).unwrap_or(path)
}

fn host(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit('@').next()?;
    (!host.is_empty()).then_some(host)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_shell_command_is_shown_whole() {
        let command = "cd app &&\nnpm test -- --watch=false";
        let t = approval_text(&json!({
            "tool_name": "Bash",
            "tool_input": { "command": command, "description": "Corre las pruebas" }
        }));
        assert_eq!(t.title, "Ejecutar un comando");
        assert_eq!(t.summary, "cd app &&…");
        assert!(t.detail.starts_with("Descripción: Corre las pruebas\n\n"), "got {}", t.detail);
        assert!(t.detail.contains(command), "the full command must be in the detail");
        assert!(t.can_allow);
    }

    #[test]
    fn file_tools_name_the_file_and_show_every_argument() {
        let t = approval_text(&json!({
            "tool_name": "Edit",
            "tool_input": { "file_path": "/Users/a/p/src/main.rs", "old_string": "a", "new_string": "b", "replace_all": false }
        }));
        assert_eq!(t.title, "Editar un archivo");
        assert_eq!(t.summary, "main.rs");
        assert!(t.detail.starts_with("Archivo: /Users/a/p/src/main.rs"));
        // Sorted, strings on their own line, other values as JSON.
        let new = t.detail.find("new_string:\nb").unwrap();
        let old = t.detail.find("old_string:\na").unwrap();
        let all = t.detail.find("replace_all: false").unwrap();
        assert!(new < old && old < all, "got {}", t.detail);
    }

    #[test]
    fn web_and_mcp_tools_read_naturally() {
        let t = approval_text(&json!({ "tool_name": "WebFetch", "tool_input": { "url": "https://user@docs.rs/serde?x=1", "prompt": "p" } }));
        assert_eq!((t.title.as_str(), t.summary.as_str()), ("Abrir una página web", "docs.rs"));

        let t = approval_text(&json!({ "tool_name": "mcp__github__create_issue", "tool_input": { "title": "Bug" } }));
        assert_eq!(t.title, "Usar «create_issue» de github");
        assert!(t.detail.starts_with("Servidor: github\nHerramienta: create_issue\n\ntitle:\nBug"), "got {}", t.detail);

        let t = approval_text(&json!({ "tool_name": "SomethingNew" }));
        assert_eq!((t.title.as_str(), t.summary.as_str(), t.detail.as_str()), ("Usar SomethingNew", "SomethingNew", ""));
    }

    #[test]
    fn codex_shell_commands_count_as_commands() {
        let t = approval_text(&json!({ "tool_name": "shell", "tool_input": { "command": ["bash", "-lc", "rm -rf build"] } }));
        assert_eq!(t.title, "Ejecutar un comando");
        assert_eq!(t.summary, "bash -lc rm -rf build");
    }

    #[test]
    fn a_long_summary_is_shortened_but_the_detail_is_not() {
        let command = format!("echo {}", "x".repeat(300));
        let t = approval_text(&json!({ "tool_name": "Bash", "tool_input": { "command": command } }));
        assert_eq!(t.summary.chars().count(), SUMMARY_LEN + 1);
        assert!(t.summary.ends_with('…'));
        assert_eq!(t.detail, command);
    }

    #[test]
    fn a_truncated_request_cannot_be_allowed() {
        let t = approval_text(&json!({ "tool_name": "Bash", "tool_input": { "command": "ls…" }, "_truncated": true }));
        assert!(!t.can_allow);
        assert!(t.detail.contains("recortada"));
    }
}
