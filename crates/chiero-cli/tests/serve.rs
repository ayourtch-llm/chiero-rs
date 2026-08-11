//! Covers: 050 contract 18 — the CLI and the JSON-RPC surface expose the same operations with
//! the same names, checked mechanically.
//!
//! **`chiero serve` — the JSON-RPC surface, and 050 contract 18's other half.**
//!
//! 080 has said *"M7 🟡 — 10 operations, all reachable as `chiero <op>`; **no MCP or JSON-RPC
//! server yet**, so contract 18's CLI/MCP identity check cannot run"* for as long as it has had
//! marks on it, and §9.1 never carried it as an item until 2026-08-11 (item 6a). This is the
//! first slice: newline-delimited JSON-RPC 2.0 on stdin/stdout, `tools/list` and `tools/call`.
//!
//! **Named for what it is.** This is JSON-RPC, not the full MCP handshake — no `initialize`
//! lifecycle, no content blocks, no notifications. 050 §3 asks for both and this is the half
//! that makes contract 18 checkable; claiming the other half before it exists is the failure
//! this project spends its time preventing.
//!
//! # Why parity is structural here
//!
//! Contract 18 wants the two surfaces to expose *the same operations with the same names*,
//! checked mechanically. The server dispatches through the same table `--help` renders from
//! (`src/help.rs`), so the two cannot drift by construction — and this file asserts it anyway,
//! because "cannot drift by construction" is a claim about code that changes.

use std::io::Write as _;
use std::process::{Command, Stdio};

const MAIN: &str = include_str!("../src/main.rs");

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_chiero")
}

/// Feed `requests` to `chiero serve` on stdin, one JSON object per line, and collect the
/// responses.
fn serve(requests: &[&str]) -> Vec<serde_json::Value> {
    let mut child = Command::new(bin())
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("cannot run `{} serve`: {e}", bin()));
    {
        let stdin = child.stdin.as_mut().expect("piped");
        for r in requests {
            writeln!(stdin, "{r}").expect("write");
        }
    }
    let out = child.wait_with_output().expect("wait");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "`serve` exited {:?}\nstdout: {text}\nstderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            serde_json::from_str(l).unwrap_or_else(|e| panic!("response is not JSON ({e}): {l}"))
        })
        .collect()
}

/// The operations the CLI dispatches, read from `run`'s `match` — the same source
/// `tests/help.rs` uses, so this file cannot be satisfied by a hand-kept list either.
fn cli_operations() -> Vec<String> {
    let start = MAIN
        .find("let env = match args[0].as_str() {")
        .expect("the dispatch match moved");
    let body = &MAIN[start..];
    let end = body
        .find("other =>")
        .expect("the dispatch match has no fallback");
    let mut names = vec!["cir".to_string()];
    for line in body[..end].lines() {
        if let Some(rest) = line.trim().strip_prefix('"')
            && let Some((name, tail)) = rest.split_once('"')
            && tail.contains("=>")
        {
            names.push(name.to_string());
        }
    }
    names.sort();
    names
}

#[test]
fn tools_list_offers_exactly_the_operations_the_cli_dispatches() {
    let responses = serve(&[r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#]);
    assert_eq!(
        responses.len(),
        1,
        "one request, one response: {responses:?}"
    );
    let r = &responses[0];
    assert_eq!(r["jsonrpc"], "2.0", "{r:#}");
    assert_eq!(r["id"], 1, "{r:#}");
    let tools = r["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("no tools array:\n{r:#}"));
    let mut names: Vec<String> = tools
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_owned))
        .collect();
    names.sort();
    // **050 contract 18, mechanically.** Not a fixed list: both sides are read from the code.
    assert_eq!(
        names,
        cli_operations(),
        "the served operations and the CLI's dispatch have drifted"
    );
    for t in tools {
        assert!(
            t["description"].as_str().is_some_and(|d| d.len() > 20),
            "every tool needs a description a caller can choose by:\n{t:#}"
        );
    }
}

/// A malformed request is answered, not ignored — and the connection survives it.
#[test]
fn a_bad_request_gets_an_error_object_and_the_server_keeps_going() {
    let responses = serve(&[
        "{not json at all",
        r#"{"jsonrpc":"2.0","id":2,"method":"no/such/method"}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#,
    ]);
    assert_eq!(responses.len(), 3, "every line gets a reply: {responses:?}");
    assert!(
        responses[0]["error"]["code"].is_i64(),
        "a parse failure is a JSON-RPC error object:\n{:#}",
        responses[0]
    );
    assert_eq!(
        responses[1]["error"]["code"], -32601,
        "an unknown method is -32601 Method not found:\n{:#}",
        responses[1]
    );
    assert!(
        responses[2]["result"]["tools"].is_array(),
        "the third request must still be served — a bad line kills the request, not the \
         session:\n{:#}",
        responses[2]
    );
}

/// **`tools/call` runs the operation and returns its envelope.**
///
/// The argument shape is the command line, because that is what `tools/list` advertises under
/// `usage` and what `Options::parse` already reads. A second argument grammar would be a second
/// parser, and 050 §1's "thin wrapper" rule is about exactly that: the one thing in this system
/// that must not have two implementations is the answer.
#[test]
fn tools_call_runs_the_operation_and_returns_its_envelope() {
    let d = std::env::temp_dir().join(format!("chiero-serve-{}", std::process::id()));
    std::fs::create_dir_all(&d).expect("scratch");
    let c = d.join("t.c");
    std::fs::write(&c, "struct s { char a; int b; char c; };\n").expect("write");
    let req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "tools/call",
        "params": { "name": "layout", "arguments": [c.display().to_string(), "--no-system-headers"] },
    })
    .to_string();
    let responses = serve(&[&req]);
    let r = &responses[0];
    assert_eq!(r["id"], 7, "{r:#}");
    // MCP: the structured half rides in `structuredContent`, which the schema describes as
    // "an optional JSON object that represents the structured result of the tool call".
    let env = &r["result"]["structuredContent"]["envelope"];
    assert!(
        env["result"]["records"].is_array(),
        "the envelope must be the operation's own, whole:\n{r:#}"
    );
    // 050 §2: the qualification travels with the answer on every surface, not just the CLI's.
    for k in ["fidelity", "proven", "assumptions", "blind_spots"] {
        assert!(
            env.get(k).is_some(),
            "the envelope reached the caller without `{k}` — the whole point of it:\n{r:#}"
        );
    }
}

/// An operation that refuses says so as a JSON-RPC error, and the session survives.
#[test]
fn a_failing_operation_is_an_error_object_not_a_crash() {
    let req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 8,
        "method": "tools/call",
        "params": { "name": "layout", "arguments": ["/nonexistent/nope.c"] },
    })
    .to_string();
    let after = r#"{"jsonrpc":"2.0","id":9,"method":"tools/list"}"#;
    let responses = serve(&[&req, after]);
    // Since the surface speaks MCP, a tool that ran and refused is a *result* with `isError`
    // rather than a protocol error — see `an_operation_that_refuses_is_an_error_result_not_a_
    // protocol_error`. What must not change is that the reason names the file.
    assert!(
        responses[0]["result"]["content"][0]["text"]
            .as_str()
            .is_some_and(|m| m.contains("nope.c")),
        "the failure must name the file, as the CLI's does:\n{:#}",
        responses[0]
    );
    assert!(
        responses[1]["result"]["tools"].is_array(),
        "one operation's failure must not end the session:\n{:#}",
        responses[1]
    );
}

/// A tool nobody offers is refused by name.
#[test]
fn an_unknown_tool_is_refused_and_says_which() {
    let req = r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"nope","arguments":[]}}"#;
    let responses = serve(&[req]);
    // **A tool nobody offers is a protocol error, not an `isError` result** — the caller asked
    // for something that does not exist, which is a fact about the request rather than an
    // outcome of running anything.
    assert!(
        responses[0]["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("nope")),
        "{:#}",
        responses[0]
    );
}

/// **A mode nobody can find is a mode nobody has.** `serve` is not an operation — it takes no
/// `<file.c>`, answers no question about a program, and must stay out of the catalogue
/// `tools/list` renders — so it needs its own line in the global help rather than a row in the
/// operations table.
#[test]
fn the_global_help_says_serve_exists() {
    let out = std::process::Command::new(bin())
        .arg("--help")
        .output()
        .expect("run");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        text.contains("chiero serve"),
        "`--help` never mentions the JSON-RPC surface:\n{text}"
    );
    assert!(
        text.contains("JSON-RPC"),
        "and it must say what the mode speaks, since it is not MCP:\n{text}"
    );
    // And it must NOT be an operation: the catalogue is what `tools/list` serves.
    let listed = serve(&[r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#]);
    let names: Vec<&str> = listed[0]["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(
        !names.contains(&"serve"),
        "`serve` is a mode, not an operation, and offering it as a tool would invite a caller \
         to recurse into it: {names:?}"
    );
}

/// The vendored MCP schema — the oracle, read rather than remembered.
///
/// **Not a full JSON-Schema validation**: there is no validator crate in this tree and 001 §4
/// says not to add one for this. What it does instead is take each definition's own `required`
/// list out of the schema and assert those keys are present, so the *shape* this surface claims
/// cannot drift from the specification even though the types are unchecked. Partial, and
/// honest about which part.
fn mcp_required(definition: &str) -> Vec<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/corpus/mcp/schema-2025-06-18.json");
    let text = std::fs::read_to_string(&path).expect("the vendored schema");
    let doc: serde_json::Value = serde_json::from_str(&text).expect("schema parses");
    doc["definitions"][definition]["required"]
        .as_array()
        .unwrap_or_else(|| panic!("no `required` for {definition} in the schema"))
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect()
}

#[test]
fn initialize_answers_with_every_field_the_schema_requires() {
    let responses = serve(&[
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#,
    ]);
    let r = &responses[0]["result"];
    for key in mcp_required("InitializeResult") {
        assert!(
            r.get(&key).is_some(),
            "`initialize` answered without `{key}`, which the schema marks required:\n{r:#}"
        );
    }
    for key in mcp_required("Implementation") {
        assert!(
            r["serverInfo"].get(&key).is_some(),
            "`serverInfo` is an Implementation and needs `{key}`:\n{r:#}"
        );
    }
    assert_eq!(
        r["protocolVersion"], "2025-06-18",
        "the version must be the one whose schema is vendored, or the oracle is not the oracle"
    );
}

#[test]
fn every_tool_has_the_fields_the_schema_requires() {
    let responses = serve(&[r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#]);
    let tools = responses[0]["result"]["tools"].as_array().expect("tools");
    let required = mcp_required("Tool");
    for t in tools {
        for key in &required {
            assert!(
                t.get(key).is_some(),
                "a tool without `{key}`, which the schema marks required:\n{t:#}"
            );
        }
        assert_eq!(
            t["inputSchema"]["type"], "object",
            "an inputSchema is a JSON Schema object:\n{t:#}"
        );
    }
}

#[test]
fn a_tool_call_answers_in_the_shape_the_schema_requires() {
    let d = std::env::temp_dir().join(format!("chiero-serve-mcp-{}", std::process::id()));
    std::fs::create_dir_all(&d).expect("scratch");
    let c = d.join("t.c");
    std::fs::write(&c, "struct s { char a; int b; };\n").expect("write");
    let req = serde_json::json!({
        "jsonrpc": "2.0", "id": 4, "method": "tools/call",
        "params": { "name": "layout", "arguments": [c.display().to_string(), "--no-system-headers"] },
    })
    .to_string();
    let r = &serve(&[&req])[0]["result"];
    for key in mcp_required("CallToolResult") {
        assert!(
            r.get(&key).is_some(),
            "a tools/call result without `{key}`, which the schema marks required:\n{r:#}"
        );
    }
    // **`content` is the unstructured half and must actually say something.** A client that
    // renders only content — which the schema permits, since `structuredContent` is optional —
    // must still learn the answer rather than being handed an empty list.
    let content = r["content"].as_array().expect("content is an array");
    assert!(!content.is_empty(), "empty content:\n{r:#}");
    assert_eq!(content[0]["type"], "text", "{r:#}");
    let text = content[0]["text"].as_str().expect("text");
    // **`Envelope::render`'s own vocabulary, not the JSON's.** The human rendering says
    // "proven — this holds for all inputs (Exact)" or "not proven — within this run's bounds";
    // it does not use the word "fidelity", and asserting on the JSON's field names here was this
    // test demanding a rendering the tool does not produce.
    assert!(
        text.contains("proven"),
        "the rendering must carry the envelope's qualification, or a content-only client is \
         reading a bare answer — the one thing 050 §2 forbids:\n{text}"
    );
    assert!(
        r["structuredContent"]["envelope"]["result"].is_object(),
        "and the structured half is the envelope itself:\n{r:#}"
    );
}

/// A failed operation is `isError`, not a JSON-RPC error, once a client is speaking MCP.
///
/// ⚠️ **This is a real difference and it is the schema's, not a preference.** MCP reserves
/// protocol errors for protocol problems; a tool that ran and refused is a *result* with
/// `isError: true`, so a client can show the reason instead of treating the session as broken.
#[test]
fn an_operation_that_refuses_is_an_error_result_not_a_protocol_error() {
    let req = serde_json::json!({
        "jsonrpc": "2.0", "id": 5, "method": "tools/call",
        "params": { "name": "layout", "arguments": ["/nonexistent/nope.c"] },
    })
    .to_string();
    let r = &serve(&[&req])[0];
    assert!(
        r.get("error").is_none(),
        "a tool that ran and refused is not a protocol failure:\n{r:#}"
    );
    assert_eq!(r["result"]["isError"], true, "{r:#}");
    let text = r["result"]["content"][0]["text"].as_str().expect("text");
    assert!(
        text.contains("nope.c"),
        "the reason must name the file:\n{text}"
    );
}
