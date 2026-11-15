// SPDX-License-Identifier: Apache-2.0
//! Asking for a signature (ADR-0004).
//!
//! A request to a destination granted with `--sign` is signed before it
//! leaves. The signing is done in Almide (`src/signer.almd`), which this side
//! cannot call, so nautra is started once more as `nautra __sign` and asked
//! over a pipe: one line of JSON for a request, one line back. The copy is
//! started at the first request that needs it and kept until nautra ends,
//! when its input closes and it ends too.
//!
//! The copy finds the credentials itself. Nothing here holds a key.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

use serde_json::Value;

struct Copy {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

/// One copy for the whole process: a question carries everything it needs,
/// so runs served side by side can share it, one question at a time.
static COPY: Mutex<Option<Copy>> = Mutex::new(None);

fn start() -> Result<Copy, String> {
    let program = std::env::current_exe().map_err(|error| format!("cannot find nautra's own binary to sign with: {error}"))?;
    let mut child = Command::new(&program)
        .arg("__sign")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("cannot start {}: {error}", program.display()))?;
    match (child.stdin.take(), child.stdout.take()) {
        (Some(input), Some(output)) => Ok(Copy { child, input, output: BufReader::new(output) }),
        _ => Err("cannot talk to nautra's signing copy".into()),
    }
}

fn exchange(copy: &mut Copy, question: &str) -> std::io::Result<String> {
    copy.input.write_all(question.as_bytes())?;
    copy.input.write_all(b"\n")?;
    copy.input.flush()?;
    let mut answer = String::new();
    if copy.output.read_line(&mut answer)? == 0 {
        return Err(std::io::ErrorKind::UnexpectedEof.into());
    }
    Ok(answer)
}

/// The headers that sign the request `question` describes, or why there are
/// none. Blocks while the copy answers.
pub(super) fn ask(question: &Value) -> Result<Vec<(String, String)>, String> {
    // serde_json writes no newline inside a document, so it is one line.
    let line = question.to_string();
    let mut held = COPY.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    // A copy that has gone away is started again, once.
    let mut answer = None;
    for _ in 0..2 {
        if held.is_none() {
            *held = Some(start()?);
        }
        let Some(copy) = held.as_mut() else { break };
        match exchange(copy, &line) {
            Ok(text) => {
                answer = Some(text);
                break;
            }
            Err(_) => {
                let _ = copy.child.kill();
                let _ = copy.child.wait();
                *held = None;
            }
        }
    }
    let answer = answer.ok_or("nautra's signing copy stopped answering")?;
    let answer: Value = serde_json::from_str(&answer).map_err(|_| "nautra's signing copy gave no answer")?;
    if let Some(reason) = answer["error"].as_str() {
        return Err(reason.to_owned());
    }
    let headers = answer["headers"].as_array().ok_or("nautra's signing copy gave no headers")?;
    Ok(headers.iter().filter_map(|pair| Some((pair[0].as_str()?.to_owned(), pair[1].as_str()?.to_owned()))).collect())
}
