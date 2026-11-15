// SPDX-License-Identifier: Apache-2.0
//! Asking how a request is authenticated (ADR-0004).
//!
//! A request to a destination granted with `--auth` is authenticated, as the
//! host, before it leaves. How is a scheme's business, and schemes are
//! written in Almide (`src/authenticator.almd`), which this side cannot
//! call. So nautra is started once more as `nautra __auth` and asked over a
//! pipe: one line of JSON describing a request, one line back saying which
//! headers to drop and which to put on. The copy is started at the first
//! request that needs it and kept until nautra ends, when its input closes
//! and it ends too.
//!
//! Nothing here knows a scheme, and nothing here holds a key: the copy finds
//! the host's credentials itself.

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

/// What to do to a request: drop `remove`, then put on `set`.
pub(super) struct Answer {
    pub(super) remove: Vec<String>,
    pub(super) set: Vec<(String, String)>,
}

fn start() -> Result<Copy, String> {
    let program = std::env::current_exe().map_err(|error| format!("cannot find nautra's own binary to ask: {error}"))?;
    let mut child = Command::new(&program)
        .arg("__auth")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("cannot start {}: {error}", program.display()))?;
    match (child.stdin.take(), child.stdout.take()) {
        (Some(input), Some(output)) => Ok(Copy { child, input, output: BufReader::new(output) }),
        _ => Err("cannot talk to nautra's answering copy".into()),
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

/// How to authenticate the request `question` describes, or why it cannot
/// be. Blocks while the copy answers.
pub(super) fn ask(question: &Value) -> Result<Answer, String> {
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
    let answer = answer.ok_or("nautra's answering copy stopped answering")?;
    let answer: Value = serde_json::from_str(&answer).map_err(|_| "nautra's answering copy gave no answer")?;
    if let Some(reason) = answer["error"].as_str() {
        return Err(reason.to_owned());
    }
    let set = answer["set"].as_array().ok_or("nautra's answering copy gave no headers")?;
    Ok(Answer {
        remove: answer["remove"].as_array().into_iter().flatten().filter_map(|name| Some(name.as_str()?.to_owned())).collect(),
        set: set.iter().filter_map(|pair| Some((pair[0].as_str()?.to_owned(), pair[1].as_str()?.to_owned()))).collect(),
    })
}
