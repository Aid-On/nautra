// SPDX-License-Identifier: Apache-2.0
//! Starting nautra again in the background, for `run -d`.
//!
//! There is no daemon: a detached run is this same program, started once more
//! with its output going to a file and no terminal attached. This can start
//! nothing but nautra itself — the program is this executable, not a path the
//! caller chooses.

use std::fs::OpenOptions;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use serde_json::{json, Value};

/// Request: `{"args": […], "log": path}`. Reply: `{"pid": N}` or `{"error"}`.
/// The child gets no stdin, appends stdout and stderr to the log, and leads
/// its own process group, so a Ctrl-C in the starting terminal does not
/// reach it.
pub fn nt_spawn_self(request_json: &str) -> String {
    let reply = match serde_json::from_str::<Value>(request_json) {
        Ok(request) => spawn(&request).unwrap_or_else(|reason| json!({ "error": reason })),
        Err(error) => json!({ "error": format!("malformed spawn request: {error}") }),
    };
    reply.to_string()
}

fn spawn(request: &Value) -> Result<Value, String> {
    let log = request["log"].as_str().ok_or("spawn request has no log")?;
    let args: Vec<&str> = request["args"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let program = std::env::current_exe().map_err(|error| format!("cannot find this executable: {error}"))?;
    let out = OpenOptions::new().create(true).append(true).open(log).map_err(|error| format!("cannot open {log}: {error}"))?;
    let err = out.try_clone().map_err(|error| format!("cannot open {log}: {error}"))?;
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .process_group(0)
        .spawn()
        .map_err(|error| format!("cannot start in the background: {error}"))?;
    Ok(json!({ "pid": child.id() }))
}
