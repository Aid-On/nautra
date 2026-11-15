// SPDX-License-Identifier: Apache-2.0
//! The record of a run: every exchange at the guest's boundary, in order, so
//! that the run can be replayed without the world, or picked up where it
//! stopped.
//!
//! An exchange is something the guest asked for and what came back: an HTTP
//! request and its response, a tool call and its result. Each is written as
//! two lines — what was asked, before it is carried out, and what came back,
//! after — so a run that stopped in between is known to have stopped there.
//!
//!   {"record": 1, "wasm": "sha256:…", "run": […], "cwd": "…"}
//!   {"seq": 1, "begin": {…}}
//!   {"seq": 1, "end": {…}}
//!   {"exit": 0}
//!
//! A record holds what the guest received — a model's replies, a tool's
//! output — and so is as private as the run was. It never holds a secret:
//! request headers are not recorded.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::future::Future;
use std::io::Write;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// A new run: every exchange is carried out and written down.
    Record,
    /// Nothing is carried out: every exchange is answered from the record.
    Replay,
    /// Answered from the record while it lasts, then carried out and written.
    Resume,
}

struct State {
    mode: Mode,
    /// Recorded exchanges not yet reached: what was asked, and what came
    /// back if the run got that far.
    ahead: VecDeque<(Value, Option<Value>)>,
    file: Option<File>,
    seq: u64,
    replayed: u64,
    recorded_exit: Option<i64>,
    /// Why the run can no longer be trusted to match its record.
    fault: Option<String>,
}

pub(super) struct Journal(Mutex<State>);

impl Journal {
    /// Opens the record a run request names, or `None` when it names none.
    pub(super) fn of(request: &Value) -> Result<Option<Arc<Journal>>, String> {
        let journal = &request["journal"];
        let Some(path) = journal["path"].as_str().filter(|path| !path.is_empty()) else { return Ok(None) };
        let mode = match journal["mode"].as_str().unwrap_or("record") {
            "record" => Mode::Record,
            "replay" => Mode::Replay,
            "resume" => Mode::Resume,
            other => return Err(format!("unknown record mode '{other}'")),
        };
        let mut state = State { mode, ahead: VecDeque::new(), file: None, seq: 0, replayed: 0, recorded_exit: None, fault: None };
        if mode == Mode::Record {
            // A record is never written over: it is someone's evidence.
            let mut file = OpenOptions::new().create_new(true).append(true).open(path).map_err(|error| format!("cannot start the record {path}: {error}"))?;
            let mut header = journal["header"].clone();
            header["record"] = json!(1);
            writeln!(file, "{header}").map_err(|error| format!("cannot write {path}: {error}"))?;
            state.file = Some(file);
        } else {
            let text = std::fs::read_to_string(path).map_err(|error| format!("cannot read the record {path}: {error}"))?;
            state.load(&text, &journal["header"]["wasm"])?;
            if mode == Mode::Resume {
                if state.recorded_exit.is_some() {
                    return Err("this run already finished; `nautra replay` re-runs it from its record".into());
                }
                state.file = Some(OpenOptions::new().append(true).open(path).map_err(|error| format!("cannot continue the record {path}: {error}"))?);
            }
        }
        Ok(Some(Arc::new(Journal(Mutex::new(state)))))
    }

    /// One exchange. `begin` is what the guest asked for; `live` carries it
    /// out. While the record has exchanges ahead, the next one must be this
    /// same request, and its recorded answer is returned without `live`
    /// being run. An answer of `{"fault": …}` means the run has left its
    /// record.
    pub(super) async fn exchange<F>(&self, begin: Value, live: F) -> Value
    where
        F: Future<Output = Value>,
    {
        enum Next {
            Recorded(Value),
            Live(u64),
            Fault(String),
        }
        let next = {
            let mut state = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(fault) = &state.fault {
                Next::Fault(fault.clone())
            } else if let Some((asked, answer)) = state.ahead.pop_front() {
                state.seq += 1;
                if asked != begin {
                    let at = state.seq;
                    Next::Fault(state.fail(format!("at exchange {at}, the record has {} but the run asked for {}", brief(&asked), brief(&begin))))
                } else if let Some(answer) = answer {
                    state.replayed += 1;
                    Next::Recorded(answer)
                } else {
                    Next::Fault(state.fail(format!("the recorded run stopped while {} was in flight; whether it took effect is unknown, so it is not repeated", brief(&asked))))
                }
            } else if state.mode == Mode::Replay {
                Next::Fault(state.fail(format!("the run asked for {} after the record ends", brief(&begin))))
            } else {
                state.seq += 1;
                let seq = state.seq;
                state.write(json!({ "seq": seq, "begin": begin }));
                Next::Live(seq)
            }
        };
        match next {
            Next::Recorded(answer) => answer,
            Next::Fault(reason) => json!({ "fault": reason }),
            Next::Live(seq) => {
                let answer = live.await;
                let mut state = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                state.write(json!({ "seq": seq, "end": answer }));
                answer
            }
        }
    }

    /// Closes the record when the guest has ended. A replay is checked here:
    /// it must have used the whole record and ended the same way.
    pub(super) fn finish(&self, exit_code: i64) -> Result<(), String> {
        let mut state = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(fault) = &state.fault {
            return Err(fault.clone());
        }
        if state.mode == Mode::Replay {
            if !state.ahead.is_empty() {
                return Err(format!("the run ended with {} recorded exchange(s) not reached", state.ahead.len()));
            }
            return match state.recorded_exit {
                Some(recorded) if recorded != exit_code => Err(format!("the recorded run ended with exit code {recorded}, this one with {exit_code}")),
                _ => Ok(()),
            };
        }
        state.write(json!({ "exit": exit_code }));
        Ok(())
    }

    /// How many exchanges were answered from the record.
    pub(super) fn replayed(&self) -> u64 {
        self.0.lock().map(|state| state.replayed).unwrap_or(0)
    }
}

impl State {
    fn fail(&mut self, reason: String) -> String {
        self.fault = Some(reason.clone());
        reason
    }

    fn write(&mut self, line: Value) {
        if let Some(file) = &mut self.file {
            let _ = writeln!(file, "{line}");
            let _ = file.sync_data();
        }
    }

    /// Reads a record back. A last line cut short by a crash is passed over.
    fn load(&mut self, text: &str, wasm: &Value) -> Result<(), String> {
        let mut lines = text.lines();
        let header: Value = lines.next().and_then(|line| serde_json::from_str(line).ok()).ok_or("this file is not a nautra record")?;
        if header["record"] != json!(1) {
            return Err("this file is not a nautra record".into());
        }
        if !wasm.is_null() && header["wasm"] != *wasm {
            return Err(format!("the workload is not the one that was recorded (recorded {}, now {})", header["wasm"], wasm));
        }
        for line in lines {
            let Ok(entry) = serde_json::from_str::<Value>(line) else { continue };
            if let Some(code) = entry["exit"].as_i64() {
                self.recorded_exit = Some(code);
            } else if !entry["begin"].is_null() {
                self.ahead.push_back((entry["begin"].clone(), None));
            } else if !entry["end"].is_null() {
                if let Some(last) = self.ahead.back_mut() {
                    last.1 = Some(entry["end"].clone());
                }
            }
        }
        Ok(())
    }
}

/// An exchange in a few words, for saying where a run left its record.
fn brief(begin: &Value) -> String {
    match begin["kind"].as_str() {
        Some("http") => format!("{} {}", begin["method"].as_str().unwrap_or("?"), begin["url"].as_str().unwrap_or("?")),
        Some("tool") => format!("a call to tool {}", begin["name"].as_str().unwrap_or("?")),
        _ => "an exchange".into(),
    }
}
