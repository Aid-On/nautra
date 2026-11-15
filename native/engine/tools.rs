// SPDX-License-Identifier: Apache-2.0
//! Tools: other images a guest may call, offered to it as one MCP server at a
//! reserved name.
//!
//! A guest reaches its tools the way it reaches anything: an HTTP request.
//! Requests to `tools.nautra` never leave the host; they are answered here.
//! Each call starts the tool's image fresh, under the run request that was
//! prepared for that tool before the guest started — its own mounts, network,
//! secrets and ceilings, usually none — with the call's arguments as JSON on
//! its stdin and its stdout as the result.

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use super::journal::Journal;
use super::outbound::Audit;

/// The name a guest sends MCP requests to.
pub(super) const HOST: &str = "tools.nautra";

/// The largest request the endpoint reads, and the most output kept from a
/// tool.
pub(super) const MAX_BYTES: usize = 1 << 20;

/// The MCP revision answered when the guest does not ask for one.
const PROTOCOL: &str = "2025-06-18";

struct Tool {
    name: String,
    description: String,
    input: Value,
    check: jsonschema::Validator,
    request: Value,
    /// Calls this tool has left, when it has a ceiling.
    left: Mutex<Option<u64>>,
}

pub(super) struct Tools {
    tools: Vec<Tool>,
    /// Calls left across every tool, when the run has a ceiling.
    left: Mutex<Option<u64>>,
    audit: Option<Arc<Audit>>,
    journal: Option<Arc<Journal>>,
}

/// One JSON-RPC answer: an HTTP status and a body (empty for a notification).
pub(super) struct Answer {
    pub(super) status: u16,
    pub(super) body: Vec<u8>,
}

impl Tools {
    /// Reads the tools a run request declares; `None` when it declares none.
    /// Every input schema is compiled here, so a schema that is not one stops
    /// the run before the guest starts.
    pub(super) fn of(request: &Value, audit: Option<Arc<Audit>>, journal: Option<Arc<Journal>>) -> Result<Option<Arc<Tools>>, String> {
        let entries: Vec<&Value> = request["tools"].as_array().into_iter().flatten().collect();
        if entries.is_empty() {
            return Ok(None);
        }
        let mut tools = Vec::new();
        for entry in entries {
            let name = entry["name"].as_str().ok_or("a tool has no name")?.to_owned();
            let input = if entry["input"].is_object() { entry["input"].clone() } else { json!({ "type": "object" }) };
            let check = jsonschema::validator_for(&input).map_err(|error| format!("tool {name}: its input schema is not valid: {error}"))?;
            tools.push(Tool {
                description: entry["description"].as_str().unwrap_or("").to_owned(),
                left: Mutex::new(entry["max_calls"].as_u64().filter(|max| *max > 0)),
                request: entry["request"].clone(),
                name,
                input,
                check,
            });
        }
        let left = Mutex::new(request["max_tool_calls"].as_u64().filter(|max| *max > 0));
        Ok(Some(Arc::new(Tools { tools, left, audit, journal })))
    }

    /// Answers one MCP request.
    pub(super) async fn answer(&self, body: &[u8]) -> Answer {
        let Ok(message) = serde_json::from_slice::<Value>(body) else {
            return reply(&Value::Null, Err((-32700, "the request is not JSON".into())));
        };
        let id = message["id"].clone();
        if id.is_null() {
            // A notification: nothing is answered.
            return Answer { status: 202, body: Vec::new() };
        }
        let result = match message["method"].as_str().unwrap_or("") {
            "initialize" => Ok(json!({
                "protocolVersion": message["params"]["protocolVersion"].as_str().unwrap_or(PROTOCOL),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "nautra", "version": env!("CARGO_PKG_VERSION") },
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({
                "tools": self.tools.iter().map(|tool| json!({
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": tool.input,
                })).collect::<Vec<_>>(),
            })),
            "tools/call" => self.call(&message["params"]).await,
            other => Err((-32601, format!("unknown method '{other}'"))),
        };
        reply(&id, result)
    }

    async fn call(&self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params["name"].as_str().unwrap_or("");
        let Some(tool) = self.tools.iter().find(|tool| tool.name == name) else {
            self.record(name, "deny", json!({ "reason": "no such tool" }));
            return Err((-32602, format!("no tool named '{name}'")));
        };
        let arguments = if params["arguments"].is_null() { json!({}) } else { params["arguments"].clone() };
        // What is wrong with the arguments is said in full: the caller already
        // has the schema, and a refusal that only says "invalid" gets the same
        // call sent again.
        let violations: Vec<String> = tool
            .check
            .iter_errors(&arguments)
            .take(8)
            .map(|error| format!("arguments{}: {error}", error.instance_path()))
            .collect();
        if !violations.is_empty() {
            self.record(name, "deny", json!({ "reason": "invalid arguments" }));
            return Ok(failed(format!("invalid arguments for {name}:\n{}", violations.join("\n"))));
        }
        if !spend(&self.left) {
            self.record(name, "deny", json!({ "reason": "the run's tool calls are used up" }));
            return Ok(failed("this run has used up its tool calls".into()));
        }
        if !spend(&tool.left) {
            self.record(name, "deny", json!({ "reason": "the tool's calls are used up" }));
            return Ok(failed(format!("{name} has used up its calls for this run")));
        }
        // Under a record, the tool is run once, when the run is first made;
        // a replay is given the result it produced then.
        let live = self.run(tool, arguments.clone());
        let result = match &self.journal {
            Some(journal) => journal.exchange(json!({ "kind": "tool", "name": name, "arguments": arguments }), live).await,
            None => live.await,
        };
        Ok(match result["fault"].as_str() {
            Some(fault) => failed(format!("this run has left its record: {fault}")),
            None => result,
        })
    }

    /// Runs the tool's image and turns how it ended into an MCP result.
    async fn run(&self, tool: &Tool, arguments: Value) -> Value {
        let name = &tool.name;
        let outcome = run_tool(tool.request.clone(), arguments.to_string().into_bytes()).await;
        self.record(name, "allow", json!({ "exit_code": outcome.exit_code }));
        if outcome.exit_code == 0 {
            json!({ "content": [{ "type": "text", "text": outcome.stdout.trim_end() }], "isError": false })
        } else {
            let said = [outcome.error.as_str(), outcome.stderr.trim_end(), outcome.stdout.trim_end()]
                .into_iter()
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            failed(if said.is_empty() { format!("{name} ended with exit code {}", outcome.exit_code) } else { said })
        }
    }

    fn record(&self, name: &str, decision: &str, mut more: Value) {
        if let Some(audit) = &self.audit {
            more["kind"] = json!("tool");
            more["decision"] = json!(decision);
            more["name"] = json!(name);
            audit.record(more);
        }
    }
}

/// Takes one from a budget; false when it is already empty.
fn spend(budget: &Mutex<Option<u64>>) -> bool {
    let Ok(mut left) = budget.lock() else { return false };
    match left.as_mut() {
        None => true,
        Some(0) => false,
        Some(remaining) => {
            *remaining -= 1;
            true
        }
    }
}

/// A tool result that says the call did not work, as the caller's own data
/// rather than a protocol error, so that a model can read it and try again.
fn failed(text: String) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": true })
}

fn reply(id: &Value, result: Result<Value, (i64, String)>) -> Answer {
    let body = match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
    };
    Answer { status: 200, body: body.to_string().into_bytes() }
}

/// Runs one tool to its end on a thread of its own, so that the guest that
/// called it keeps its place in the host's event loop while it waits.
async fn run_tool(request: Value, stdin: Vec<u8>) -> super::Captured {
    let (done, wait) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let _ = done.send(super::run_captured(&request, stdin));
    });
    wait.await.unwrap_or_else(|_| super::Captured {
        exit_code: 125,
        error: "the tool's run was lost".into(),
        stdout: String::new(),
        stderr: String::new(),
    })
}
