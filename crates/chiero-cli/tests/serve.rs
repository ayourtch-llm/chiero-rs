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
