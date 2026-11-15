// SPDX-License-Identifier: Apache-2.0
//! A tool or a check that is a native command rather than a Wasm image.
//!
//! Nautra confines Wasm; it does not confine native processes. A native
//! command is handed to Porta, nautra's sister, which confines it with the
//! host kernel. The whole Porta command line is decided in Almide
//! (`src/native.almd`) before the run starts; this only starts it, gives it
//! the call's arguments on stdin, and collects what it wrote.

use std::io::{Read, Write};
use std::process::{Command, Stdio};

use serde_json::Value;

use super::tools::MAX_BYTES;
use super::Captured;

/// Whether a prepared request is for a native command.
pub(super) fn is_native(request: &Value) -> bool {
    request["native"].is_object()
}

pub(super) fn run_native(request: &Value, stdin: Vec<u8>) -> Captured {
    let refused = |error: String| Captured { exit_code: 125, error, stdout: String::new(), stderr: String::new() };
    let Some(program) = request["native"]["program"].as_str() else { return refused("a native request names no program".into()) };
    let args: Vec<&str> = request["native"]["args"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let mut child = match Command::new(program).args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(child) => child,
        Err(error) => return refused(format!("cannot start {program}: {error}")),
    };
    // Written from a thread of its own: a command that fills its output
    // before reading its input would otherwise stop both sides.
    let input = child.stdin.take().map(|mut pipe| {
        std::thread::spawn(move || {
            let _ = pipe.write_all(&stdin);
        })
    });
    let err_pipe = child.stderr.take();
    let errors = std::thread::spawn(move || read_capped(err_pipe));
    let stdout = read_capped(child.stdout.take());
    let stderr = errors.join().unwrap_or_default();
    if let Some(input) = input {
        let _ = input.join();
    }
    let exit_code = match child.wait() {
        Ok(status) => status.code().map(i64::from).unwrap_or(137),
        Err(error) => return refused(format!("lost track of {program}: {error}")),
    };
    Captured { exit_code, error: String::new(), stdout, stderr }
}

/// Reads a stream to its end, keeping at most `MAX_BYTES` of it.
fn read_capped(pipe: Option<impl Read>) -> String {
    let Some(mut pipe) = pipe else { return String::new() };
    let mut kept = Vec::new();
    let mut chunk = [0u8; 8192];
    while let Ok(read) = pipe.read(&mut chunk) {
        if read == 0 {
            break;
        }
        let room = MAX_BYTES.saturating_sub(kept.len());
        kept.extend_from_slice(&chunk[..read.min(room)]);
    }
    String::from_utf8_lossy(&kept).into_owned()
}
