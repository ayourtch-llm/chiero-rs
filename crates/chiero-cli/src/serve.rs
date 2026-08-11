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

/// JSON-RPC 2.0's own codes. Only the ones this surface can produce — which stopped including
/// `-32603 Internal error` when a failed *tool* became an `isError` result rather than a
/// protocol failure. The compiler noticed before I did.
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
                // **The command line, verbatim.** `usage` is not an MCP field; it is here
                // because it is what a caller needs to fill `arguments` in, and the alternative
                // — a JSON Schema describing each operation's flags — would be a second
                // description of a grammar `Options::parse` already owns. Two descriptions of
                // one thing is what `tests/help.rs` exists to prevent.
                "usage": format!("chiero {name} {args}"),
                // **Required by MCP, and deliberately shallow.** The tool takes a command line;
                // saying so is true, and enumerating each operation's flags here would be the
                // second parser again. A caller reads `usage` to build the array.
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "arguments": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "The command line after the operation name, as `usage` shows it.",
                        }
                    },
                    "required": ["arguments"],
                },
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
        // **The MCP handshake's opening, answered from the vendored schema's own `required`
        // list** (`tests/corpus/mcp/`). `protocolVersion` is the version of the schema in this
        // repository: claiming a version whose shape is not the one being validated against
        // would be the overclaim the vendoring exists to prevent.
        Some("initialize") => Some(result(
            id,
            serde_json::json!({
                "protocolVersion": "2025-06-18",
                // Tools and nothing else. Resources, prompts, logging and completions are not
                // implemented, and an empty capability object is how MCP says so.
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "chiero", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Every operation answers with an envelope: `fidelity`, \
                                 `proven`, `assumptions`, `blind_spots`. An empty result is \
                                 not the same as a clean one — read the envelope.",
            }),
        )),
        Some("tools/list") => Some(result(id, tools())),
        Some("tools/call") => Some(call(id, req.get("params"))),
        Some(other) => Some(error(
            id,
            METHOD_NOT_FOUND,
            &format!(
                "no method `{other}`; this surface offers initialize, tools/list and tools/call"
            ),
        )),
        None => Some(error(id, INVALID_REQUEST, "no method")),
    }
}

/// Run one operation and hand back its envelope.
///
/// **Through `crate::run`, the same function the command line calls.** 050 §1 makes the CLI a
/// thin wrapper over the operations and contract 18 makes the two surfaces identical; the
/// cheapest way to keep both true is for the second surface to *be* the first one, called with
/// an argument vector instead of `argv`. Nothing here decides anything about the code.
///
/// The envelope is embedded as JSON rather than as text, because a caller that speaks JSON-RPC
/// wants the structure — and `--json` is what produces it, so the flag is added rather than
/// asked for.
fn call(id: serde_json::Value, params: Option<&serde_json::Value>) -> serde_json::Value {
    let Some(name) = params.and_then(|p| p.get("name")).and_then(|n| n.as_str()) else {
        return error(id, INVALID_REQUEST, "tools/call needs params.name");
    };
    if !crate::help::catalogue().iter().any(|(n, _, _)| *n == name) {
        return error(
            id,
            METHOD_NOT_FOUND,
            &format!("no tool `{name}`; tools/list has the ten this offers"),
        );
    }
    let mut argv = vec![name.to_string()];
    if let Some(args) = params
        .and_then(|p| p.get("arguments"))
        .and_then(|a| a.as_array())
    {
        for a in args {
            match a.as_str() {
                Some(s) => argv.push(s.to_string()),
                // **Refused rather than coerced.** `42` and `"42"` are the same on a command
                // line and not in JSON, and guessing which the caller meant is how a tool comes
                // to analyse a file called `true`.
                None => {
                    return error(
                        id,
                        INVALID_REQUEST,
                        "every element of `arguments` must be a string, as on a command line",
                    );
                }
            }
        }
    }
    // **Twice, and the second time is not waste.** MCP's `CallToolResult` carries an
    // unstructured `content` for a client to render and an optional `structuredContent` for one
    // that parses; `--json` gives the second and the plain rendering gives the first. A client
    // may honour either, so a surface that supplies only one is guessing which client it has.
    //
    // ⚠️ Built by *adding* `--json` to a copy rather than by filtering it out of one, because
    // the first version of this did the latter and the `--json` it filtered had been deleted
    // along with the code that pushed it: both calls then produced the rendering, the parse
    // failed silently, and `structuredContent` simply never appeared. The test caught it; the
    // shape that cannot go wrong is two explicit vectors.
    let mut json_argv = argv.clone();
    json_argv.push("--json".to_string());
    let rendered = crate::run(&argv);
    let structured = crate::run(&json_argv);
    match (rendered, structured) {
        (Ok(text), Ok(json)) => {
            let mut r = serde_json::json!({
                "content": [{ "type": "text", "text": text }],
            });
            // `cir` answers in 020's normative text and carries no envelope, by design (050 §2
            // attaches a fidelity to an answer about a *program*, and that one is about chiero),
            // so there is nothing structured to offer and the key stays absent.
            if let Ok(envelope) = serde_json::from_str::<serde_json::Value>(&json) {
                r["structuredContent"] = serde_json::json!({ "envelope": envelope });
            }
            result(id, r)
        }
        // **A tool that ran and refused is a result, not a protocol error** — the schema's
        // distinction, not a preference: MCP reserves protocol errors for protocol problems, so
        // a client can show the reason instead of treating the session as broken. The words are
        // the operation's own; "operation failed" would throw away the sentence the CLI spent
        // effort making act-on-able.
        (Err(e), _) | (_, Err(e)) => {
            let why = match e {
                crate::Fault::Usage(m) | crate::Fault::Failed(m) => m,
            };
            result(
                id,
                serde_json::json!({
                    "content": [{ "type": "text", "text": why }],
                    "isError": true,
                }),
            )
        }
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
