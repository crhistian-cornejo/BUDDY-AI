//! `buddy-hook --mcp --out <dir>` — the Office tools as a stdio MCP server, so Claude Code and Codex (run as CLIs
//! by Buddy) can create Word, Excel and PowerPoint files.
//!
//! Transport: JSON-RPC 2.0, one JSON object per line on stdin, one reply per line on stdout (the MCP stdio
//! transport). stdout carries nothing else. We answer `initialize`, `ping`, `tools/list` and `tools/call`; every
//! notification is ignored; any other method gets -32601. Bad input gets a JSON-RPC error, never a panic, and EOF
//! on stdin ends the server.
//!
//! Every file lands in `<dir>` (default `./documentos`), created 0700 on Unix when missing. The tools never write
//! anywhere else and never overwrite (see `office`).

use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::office;

/// What we answer when the client asks for a version we do not know.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
/// Versions whose shape of `initialize`, `tools/list` and `tools/call` we speak: the client's is echoed back.
const KNOWN_VERSIONS: [&str; 4] = ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];
/// Longest line we read. A request past this is answered with an error and skipped, never buffered whole.
const MAX_LINE: usize = 16 * 1024 * 1024;
/// The folder, under the current directory, used when `--out` is not given (MIKA's name for it).
const DEFAULT_FOLDER: &str = "documentos";

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// `[--out <dir>]` (also `--out=<dir>`) → the absolute documents folder. A relative folder is taken from `cwd`.
pub fn parse_args(args: &[String], cwd: &Path) -> Result<PathBuf, String> {
    let mut out: Option<PathBuf> = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let value = match arg.as_str() {
            "--out" => it.next().cloned(),
            other => match other.strip_prefix("--out=") {
                Some(v) => Some(v.to_string()),
                None => return Err(format!("argumento desconocido: {other}")),
            },
        };
        match value.filter(|v| !v.is_empty()) {
            Some(v) => out = Some(PathBuf::from(v)),
            None => return Err("falta la carpeta después de --out".into()),
        }
    }
    let dir = out.unwrap_or_else(|| PathBuf::from(DEFAULT_FOLDER));
    Ok(if dir.is_absolute() { dir } else { cwd.join(dir) })
}

/// Entry point for `buddy-hook --mcp …` (`args` without `--mcp`). Returns the process exit code.
pub fn main(args: &[String]) -> i32 {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let dir = match parse_args(args, &cwd) {
        Ok(dir) => dir,
        Err(err) => {
            eprintln!("buddy-hook --mcp: {err}. Uso: buddy-hook --mcp [--out <carpeta>]");
            return 2;
        }
    };
    // Made now so the folder is there to open; if it fails, each tool call says so instead.
    let _ = office::ensure_dir(&dir);
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    match serve(stdin.lock(), stdout.lock(), &dir) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

/// Reads requests until EOF and answers each on its own line. Returns only on EOF or a broken stream.
pub fn serve<R: BufRead, W: Write>(mut reader: R, mut writer: W, dir: &Path) -> std::io::Result<()> {
    loop {
        let mut line = Vec::new();
        let read = Read::take(&mut reader, MAX_LINE as u64 + 1).read_until(b'\n', &mut line)?;
        if read == 0 {
            return Ok(());
        }
        let reply = if line.len() > MAX_LINE && line.last() != Some(&b'\n') {
            skip_line(&mut reader)?;
            Some(error(Value::Null, INVALID_REQUEST, "Mensaje demasiado grande."))
        } else {
            handle_line(&line, dir)
        };
        if let Some(reply) = reply {
            serde_json::to_writer(&mut writer, &reply)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
    }
}

/// Discards the rest of an oversized line.
fn skip_line<R: BufRead>(reader: &mut R) -> std::io::Result<()> {
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            return Ok(());
        }
        if let Some(end) = buf.iter().position(|&b| b == b'\n') {
            reader.consume(end + 1);
            return Ok(());
        }
        let n = buf.len();
        reader.consume(n);
    }
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// One line in, at most one reply out.
fn handle_line(line: &[u8], dir: &Path) -> Option<Value> {
    let mut line = line;
    if line.starts_with(&[0xEF, 0xBB, 0xBF]) {
        line = &line[3..];
    }
    if line.iter().all(u8::is_ascii_whitespace) {
        return None;
    }
    let Ok(message) = serde_json::from_slice::<Value>(line) else {
        return Some(error(Value::Null, PARSE_ERROR, "JSON no válido."));
    };
    let Some(object) = message.as_object() else {
        // Batches left MCP in 2025-06-18; a bare value was never a request.
        return Some(error(Value::Null, INVALID_REQUEST, "Se esperaba un objeto JSON-RPC."));
    };
    // No method: a reply to something we never asked. Nothing to say.
    let method = object.get("method")?.as_str()?;
    // No id: a notification (`notifications/initialized`, `notifications/cancelled`…). Never answered.
    let id = object.get("id")?;
    if !(id.is_string() || id.is_number()) {
        return Some(error(Value::Null, INVALID_REQUEST, "El id debe ser un texto o un número."));
    }
    let id = id.clone();
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    Some(match method {
        "initialize" => result(id, initialize(&params, dir)),
        "ping" => result(id, json!({})),
        "tools/list" => result(id, json!({ "tools": office::specs() })),
        "tools/call" => match call_tool(&params, dir) {
            Ok(value) => result(id, value),
            Err(message) => error(id, INVALID_PARAMS, &message),
        },
        other => error(id, METHOD_NOT_FOUND, &format!("Método desconocido: {other}")),
    })
}

fn initialize(params: &Value, dir: &Path) -> Value {
    let asked = params["protocolVersion"].as_str().unwrap_or("");
    let version = if KNOWN_VERSIONS.contains(&asked) { asked } else { PROTOCOL_VERSION };
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "buddy", "title": "Buddy", "version": env!("CARGO_PKG_VERSION") },
        "instructions": format!(
            "Crea archivos de Word, Excel y PowerPoint en {}. Pasa solo el nombre del archivo, sin carpetas.",
            dir.display()
        ),
    })
}

/// `tools/call`: an unknown tool or a call without a name is a protocol error (`Err`); everything that goes wrong
/// inside a known tool is a result with `isError`, so the agent reads the reason and can try again.
fn call_tool(params: &Value, dir: &Path) -> Result<Value, String> {
    let Some(name) = params["name"].as_str() else { return Err("Falta el nombre de la herramienta.".into()) };
    if !office::NAMES.contains(&name) {
        return Err(format!("Herramienta desconocida: {name}"));
    }
    let empty = Value::Object(Default::default());
    let args = match &params["arguments"] {
        Value::Null => &empty,
        args @ Value::Object(_) => args,
        _ => return Ok(tool_error("Los argumentos deben ser un objeto JSON.")),
    };
    // The writers do not panic on any input we know of; if one ever did, the agent gets an error, not a dead server.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| office::run(name, args, dir)));
    Ok(match outcome {
        Ok(Ok(path)) => json!({
            "content": [{ "type": "text", "text": format!("Creado: {}", path.display()) }],
            "isError": false,
        }),
        Ok(Err(err)) => tool_error(&err.0),
        Err(_) => tool_error("Error interno al crear el archivo."),
    })
}

fn tool_error(message: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": message }], "isError": true })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::office::test_support::TempDir;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// Feeds `input` to the server and returns every reply line, parsed.
    fn exchange(input: &str, dir: &Path) -> Vec<Value> {
        let mut out = Vec::new();
        serve(input.as_bytes(), &mut out, dir).unwrap();
        String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    #[test]
    fn the_out_folder_comes_from_argv_or_defaults_to_documentos() {
        let cwd = Path::new("/home/a/proyecto");
        assert_eq!(parse_args(&[], cwd).unwrap(), cwd.join("documentos"));
        assert_eq!(parse_args(&args(&["--out", "salida"]), cwd).unwrap(), cwd.join("salida"));
        assert_eq!(parse_args(&args(&["--out=salida"]), cwd).unwrap(), cwd.join("salida"));
        let abs = std::env::temp_dir().join("docs");
        assert_eq!(parse_args(&args(&["--out", &abs.to_string_lossy()]), cwd).unwrap(), abs);
        assert!(parse_args(&args(&["--out"]), cwd).is_err());
        assert!(parse_args(&args(&["--out", ""]), cwd).is_err());
        assert!(parse_args(&args(&["--verbose"]), cwd).is_err());
    }

    #[test]
    fn a_round_trip_creates_a_document() {
        let tmp = TempDir::new("mcp");
        let dir = tmp.0.join("documentos");
        let input = [
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            r#"{"jsonrpc":"2.0","id":"tres","method":"tools/call","params":{"name":"create_document","arguments":{"file":"Informe","title":"Hola","blocks":[{"type":"paragraph","text":"Uno & dos"}]}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"ping"}"#,
        ]
        .join("\n");
        let replies = exchange(&input, &dir);
        assert_eq!(replies.len(), 4, "the notification gets no reply");

        assert_eq!(replies[0]["id"], 1);
        assert_eq!(replies[0]["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(replies[0]["result"]["serverInfo"]["name"], "buddy");
        assert!(replies[0]["result"]["capabilities"]["tools"].is_object());

        let tools = replies[1]["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["create_document", "create_spreadsheet", "create_presentation"]);
        for tool in tools {
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert!(!tool["description"].as_str().unwrap().is_empty());
        }

        assert_eq!(replies[2]["id"], "tres");
        assert_eq!(replies[2]["result"]["isError"], false);
        let text = replies[2]["result"]["content"][0]["text"].as_str().unwrap();
        let path = dir.join("Informe.docx");
        assert_eq!(text, format!("Creado: {}", path.display()));
        assert!(path.is_absolute() && path.is_file());

        assert_eq!(replies[3], json!({ "jsonrpc": "2.0", "id": 4, "result": {} }));
    }

    #[test]
    fn the_clients_known_version_is_echoed_and_an_unknown_one_is_not() {
        let tmp = TempDir::new("mcp-version");
        let replies = exchange(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-03-26\"}}\n\
             {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"1999-01-01\"}}\n",
            &tmp.0,
        );
        assert_eq!(replies[0]["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(replies[1]["result"]["protocolVersion"], PROTOCOL_VERSION);
    }

    #[test]
    fn tool_failures_are_results_and_protocol_mistakes_are_errors() {
        let tmp = TempDir::new("mcp-errors");
        let input = [
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"create_spreadsheet","arguments":{"file":"x","sheets":[]}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"rm_rf","arguments":{}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"create_presentation","arguments":[1]}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call"}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"resources/list"}"#,
            r#"{not json"#,
            r#"[{"jsonrpc":"2.0","id":6,"method":"ping"}]"#,
            r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":7,"result":{}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}"#,
            "",
            "   ",
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"create_document","arguments":{"file":"../../fuera","blocks":[{"type":"paragraph","text":"x"}]}}}"#,
        ]
        .join("\r\n");
        let replies = exchange(&input, &tmp.0);
        assert_eq!(replies.len(), 9, "{replies:#?}");

        assert_eq!(replies[0]["result"]["isError"], true);
        assert_eq!(replies[0]["result"]["content"][0]["text"], "El libro no tiene hojas (sheets).");
        assert_eq!(replies[1]["error"]["code"], INVALID_PARAMS);
        assert_eq!(replies[2]["result"]["isError"], true);
        assert_eq!(replies[3]["error"]["code"], INVALID_PARAMS);
        assert_eq!(replies[4]["error"]["code"], METHOD_NOT_FOUND);
        assert_eq!(replies[4]["id"], 5);
        assert_eq!(replies[5]["error"]["code"], PARSE_ERROR);
        assert_eq!(replies[5]["id"], Value::Null);
        assert_eq!(replies[6]["error"]["code"], INVALID_REQUEST);
        assert_eq!(replies[7]["error"]["code"], INVALID_REQUEST);
        assert_eq!(replies[8]["result"]["isError"], false);
        assert!(tmp.0.join("fuera.docx").is_file(), "a path in the name is cut down to the name");
        assert!(!tmp.0.parent().unwrap().join("fuera.docx").exists());
    }

    #[test]
    fn an_oversized_line_is_refused_without_ending_the_server() {
        let tmp = TempDir::new("mcp-big");
        let mut input = String::from("{\"x\":\"");
        input.push_str(&"a".repeat(MAX_LINE + 10));
        input.push_str("\"}\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n");
        let replies = exchange(&input, &tmp.0);
        assert_eq!(replies.len(), 2);
        assert_eq!(replies[0]["error"]["code"], INVALID_REQUEST);
        assert_eq!(replies[1]["result"], json!({}));
    }

    #[test]
    fn eof_ends_the_server_quietly() {
        let tmp = TempDir::new("mcp-eof");
        assert!(exchange("", &tmp.0).is_empty());
        // A last line without its newline is still answered.
        let replies = exchange(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#, &tmp.0);
        assert_eq!(replies.len(), 1);
    }
}
