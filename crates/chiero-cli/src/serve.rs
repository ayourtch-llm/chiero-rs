//! **`chiero serve` — newline-delimited JSON-RPC 2.0 on stdin/stdout.**
//!
//! 080's status table has said *"M7 🟡 … no MCP or JSON-RPC server yet, so contract 18's
//! CLI/MCP identity check cannot run"* since it gained marks. This is the half that makes the
//! check runnable.
//!
//! ⚠️ **JSON-RPC, not MCP, and the difference is stated rather than blurred.** There is no
//! `initialize` lifecycle, no content blocks and no notifications here. 050 §3 asks for both
//! surfaces; naming this one "MCP" because it speaks JSON over a pipe would be the overclaim
//! this project exists to refuse — the same move as a `findings: []` that means "nobody looked".
//!
//! **One table, two surfaces.** The tools come from [`crate::help::catalogue`], which is what
//! `--help` renders from, so contract 18's parity is structural: there is no second list to
//! drift. `tests/serve.rs` asserts it against the dispatch `match` regardless.
//!
//! **No async runtime.** 001 §4 keeps this tree linking almost nothing, and a line-oriented
//! server needs `std::io` and `serde_json` — both already here. Reaching for `tokio` would cost
//! the property 003 §3 is written to protect, for a loop that reads a line and writes a line.

use std::io::{BufRead as _, Write as _};

/// JSON-RPC 2.0's own codes. Only the ones this surface can produce.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;

fn error(id: serde_json::Value, code: i64, message: &str) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    })
}

fn result(id: serde_json::Value, result: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// The tools, from the one table the CLI's help also renders.
fn tools() -> serde_json::Value {
    let tools: Vec<serde_json::Value> = crate::help::catalogue()
        .into_iter()
        .map(|(name, description, args)| {
            serde_json::json!({
                "name": name,
                "description": description,
                // **The command line, verbatim, rather than a JSON schema.** A schema would be a
                // second description of the arguments, and `Options::parse` is the first; two
                // descriptions of one thing is what `tests/help.rs` exists to prevent. Until the
                // parser can *emit* a schema, saying how the operation is invoked is the honest
                // form — and it is what a caller needs to build the `arguments` array below.
                "usage": format!("chiero {name} {args}"),
            })
        })
        .collect();
    serde_json::json!({ "tools": tools })
}

/// Answer one request. `None` for a notification (no `id`), which JSON-RPC says gets no reply —
/// though this surface has no notifications yet, so it is unreachable and stated rather than
/// silently absent.
fn dispatch(req: &serde_json::Value) -> Option<serde_json::Value> {
    let id = req.get("id").cloned()?;
    if req.get("jsonrpc").and_then(|v| v.as_str()) != Some("2.0") {
        return Some(error(id, INVALID_REQUEST, "jsonrpc must be \"2.0\""));
    }
    match req.get("method").and_then(|m| m.as_str()) {
        Some("tools/list") => Some(result(id, tools())),
        Some(other) => Some(error(
            id,
            METHOD_NOT_FOUND,
            &format!("no method `{other}`; this surface offers tools/list"),
        )),
        None => Some(error(id, INVALID_REQUEST, "no method")),
    }
}

/// Read lines until EOF, answering each. **A bad line kills the request, not the session** — a
/// server that exits on the first malformed input makes a caller's parse bug look like a crash.
pub(crate) fn run() -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<serde_json::Value>(&line) {
            Ok(req) => dispatch(&req),
            // **`null` id, per JSON-RPC**: the request could not be parsed, so its id is
            // unknowable, and inventing one would attach the failure to a request that may not
            // exist.
            Err(e) => Some(error(
                serde_json::Value::Null,
                PARSE_ERROR,
                &format!("not JSON: {e}"),
            )),
        };
        if let Some(reply) = reply {
            writeln!(out, "{reply}")?;
            out.flush()?;
        }
    }
    Ok(())
}
