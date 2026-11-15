// SPDX-License-Identifier: Apache-2.0
//! The engine: one WebAssembly workload run under exactly the grants it was
//! given. Everything that decides *what* a run may do lives in Almide; this
//! file applies a finished request to wasmtime and reports how the run ended.
//!
//! A request is one JSON document (see `src/engine.almd`). A guest starts with
//! no arguments beyond its own, an empty environment, a closed stdin, no
//! directory and no network; each field of the request opens one thing.

use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use wasmtime::component::{Component, Linker as ComponentLinker, ResourceTable};
use wasmtime::{Config, Engine, Error, Linker, Module, Store, StoreLimits, StoreLimitsBuilder, Trap};
use wasmtime_wasi::p1::WasiP1Ctx;
use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};
use wasmtime_wasi::sockets::SocketAddrUse;
use wasmtime_wasi::{FsPerms, I32Exit, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpCtxView, WasiHttpView};

use journal::Journal;
use outbound::{Audit, Outbound};
use tools::Tools;

#[path = "engine/inspect.rs"]
mod inspect;
#[path = "engine/journal.rs"]
mod journal;
#[path = "engine/outbound.rs"]
mod outbound;
#[path = "engine/serve.rs"]
mod serve;
#[path = "engine/spawn.rs"]
mod spawn;
#[path = "engine/tools.rs"]
mod tools;
#[path = "engine/upload.rs"]
mod upload;

pub use inspect::nt_inspect;
pub use serve::nt_serve;
pub use spawn::nt_spawn_self;
pub use upload::nt_http_upload;

/// Exit codes for runs that did not end by the guest's own choice, after the
/// conventions container users already know: 124 for a deadline, 125 for a
/// run the engine refused or could not start, 137 for a resource ceiling.
const EXIT_TIMEOUT: i64 = 124;
const EXIT_REFUSED: i64 = 125;
/// The guest ended well, and a finish check did not accept its work.
const EXIT_NOT_ACCEPTED: i64 = 4;
/// A replayed or resumed run that did not follow its record.
const EXIT_LEFT_RECORD: i64 = 3;
const EXIT_TRAPPED: i64 = 134;
const EXIT_CEILING: i64 = 137;

/// Runs one workload to completion with this process's stdout and stderr.
/// Returns `{"exit_code": N}`, with `"error"` beside it when the run ended
/// for a reason the guest did not choose.
pub fn nt_run(request_json: &str) -> String {
    let outcome = match serde_json::from_str::<Value>(request_json) {
        Ok(request) => run(&request),
        Err(error) => refused(format!("malformed run request: {error}")),
    };
    outcome.to_string()
}

/// Parses TOML and returns it as JSON text, or `{"error": …}`. The Nautrafile
/// is TOML; the rest of nautra reads JSON values.
pub fn nt_toml_to_json(text: &str) -> String {
    match toml::from_str::<Value>(text) {
        Ok(value) => json!({ "ok": value }).to_string(),
        Err(error) => json!({ "error": error.to_string() }).to_string(),
    }
}

fn refused(reason: String) -> Value {
    json!({ "exit_code": EXIT_REFUSED, "error": reason })
}

/// One address a guest may connect to. `None` is "any".
#[derive(Clone)]
struct NetRule {
    ip: Option<IpAddr>,
    port: Option<u16>,
}

impl NetRule {
    fn admits(&self, addr: &SocketAddr) -> bool {
        self.ip.is_none_or(|ip| ip == addr.ip()) && self.port.is_none_or(|port| port == addr.port())
    }
}

struct Limits {
    memory_bytes: usize,
    fuel: u64,
    timeout_ms: u64,
}

impl Limits {
    fn of(request: &Value) -> Limits {
        Limits {
            memory_bytes: request["memory_bytes"].as_u64().unwrap_or(0) as usize,
            fuel: request["fuel"].as_u64().unwrap_or(0),
            timeout_ms: request["timeout_ms"].as_u64().unwrap_or(0),
        }
    }
}

struct ComponentCtx {
    wasi: WasiCtx,
    table: ResourceTable,
    limits: StoreLimits,
    http: WasiHttpCtx,
    outbound: Outbound,
}

impl ComponentCtx {
    fn new(mut wasi: WasiCtxBuilder, request: &Value, limits: &Limits, audit: Option<Arc<Audit>>, tools: Option<Arc<Tools>>, journal: Option<Arc<Journal>>) -> ComponentCtx {
        ComponentCtx {
            wasi: wasi.build(),
            table: ResourceTable::new(),
            limits: store_limits(limits),
            http: WasiHttpCtx::new(),
            outbound: Outbound::of(request, audit, tools, journal),
        }
    }
}

impl WasiView for ComponentCtx {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

impl WasiHttpView for ComponentCtx {
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        WasiHttpCtxView { ctx: &mut self.http, table: &mut self.table, hooks: &mut self.outbound }
    }
}

struct ModuleCtx {
    wasi: WasiP1Ctx,
    limits: StoreLimits,
}

/// A run whose standard streams are memory instead of this process's: what
/// it reads as stdin, and what it wrote.
struct Capture {
    stdin: Vec<u8>,
    out: MemoryOutputPipe,
    err: MemoryOutputPipe,
}

/// How a captured run ended and what it wrote.
struct Captured {
    exit_code: i64,
    error: String,
    stdout: String,
    stderr: String,
}

fn run(request: &Value) -> Value {
    run_with(request, None)
}

/// Runs a workload with `stdin` as its input and returns what it wrote. This
/// is how a tool is called.
fn run_captured(request: &Value, stdin: Vec<u8>) -> Captured {
    let capture = Capture { stdin, out: MemoryOutputPipe::new(tools::MAX_BYTES), err: MemoryOutputPipe::new(tools::MAX_BYTES) };
    let report = run_with(request, Some(&capture));
    Captured {
        exit_code: report["exit_code"].as_i64().unwrap_or(EXIT_REFUSED),
        error: report["error"].as_str().unwrap_or("").to_owned(),
        stdout: String::from_utf8_lossy(&capture.out.contents()).into_owned(),
        stderr: String::from_utf8_lossy(&capture.err.contents()).into_owned(),
    }
}

fn run_with(request: &Value, capture: Option<&Capture>) -> Value {
    let Some(path) = request["path"].as_str() else { return refused("run request has no path".into()) };
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => return refused(format!("cannot read {path}: {error}")),
    };
    let limits = Limits::of(request);
    let engine = match Engine::new(&engine_config(&limits)) {
        Ok(engine) => engine,
        Err(error) => return refused(format!("engine setup failed: {error}")),
    };
    let prepared = Audit::of(request).and_then(|audit| {
        let journal = Journal::of(request)?;
        let tools = Tools::of(request, audit.clone(), journal.clone())?;
        let wasi = wasi_builder(request, audit.clone(), capture)?;
        Ok((audit, journal, tools, wasi))
    });
    let (audit, journal, tools, wasi) = match prepared {
        Ok(prepared) => prepared,
        Err(reason) => return refused(reason),
    };
    let component = inspect::is_component(&bytes);
    if tools.is_some() && !component {
        return refused("tools are reached over wasi:http, which a core module does not have: build the workload as a component".into());
    }
    let deadline = Deadline::arm(&engine, limits.timeout_ms);
    let ended = if component {
        run_component(&engine, &bytes, ComponentCtx::new(wasi, request, &limits, audit, tools.clone(), journal.clone()), &limits)
    } else {
        run_module(&engine, &bytes, wasi, &limits)
    };
    drop(deadline);
    let mut report = match ended {
        Ok(result) => exit_report(result, &limits),
        Err(reason) => return refused(reason),
    };
    if let Some(journal) = &journal {
        report["replayed"] = json!(journal.replayed());
        if let Err(reason) = journal.finish(report["exit_code"].as_i64().unwrap_or(EXIT_REFUSED)) {
            return json!({ "exit_code": EXIT_LEFT_RECORD, "error": format!("the run left its record: {reason}") });
        }
    }
    // A guest saying it is done is a claim; the finish checks are what
    // accept it.
    if let Some(tools) = tools.filter(|tools| report["exit_code"] == json!(0) && tools.has_finish_checks()) {
        if let Err(objection) = tools.accept_finish() {
            return json!({ "exit_code": EXIT_NOT_ACCEPTED, "error": format!("the run ended, but its work is not accepted: {objection}") });
        }
    }
    report
}

/// Fuel and epoch checks cost time in every loop, so each is compiled in only
/// when the run asked for that ceiling. The component model's async ABI is
/// always on: a core module or a WASI 0.2 component ignores it, and a WASI 0.3
/// component cannot load without it.
fn engine_config(limits: &Limits) -> Config {
    let mut config = Config::new();
    config.consume_fuel(limits.fuel > 0);
    config.epoch_interruption(limits.timeout_ms > 0);
    config.wasm_component_model(true);
    config.wasm_component_model_async(true);
    config.wasm_component_model_more_async_builtins(true);
    config.concurrency_support(true);
    config
}

/// The deadline of a run: a thread that advances the engine's epoch once the
/// time is up, which traps the guest at its next loop or call. Dropping it
/// after the run lets the thread end without touching anything.
struct Deadline(Option<std::sync::mpsc::Sender<()>>);

impl Deadline {
    fn arm(engine: &Engine, timeout_ms: u64) -> Deadline {
        if timeout_ms == 0 {
            return Deadline(None);
        }
        let (finished, wait) = std::sync::mpsc::channel::<()>();
        let engine = engine.clone();
        std::thread::spawn(move || {
            if wait.recv_timeout(Duration::from_millis(timeout_ms)).is_err() {
                engine.increment_epoch();
            }
        });
        Deadline(Some(finished))
    }
}

impl Drop for Deadline {
    fn drop(&mut self) {
        if let Some(finished) = self.0.take() {
            let _ = finished.send(());
        }
    }
}

/// The guest's whole view of the host, built from the request and nothing
/// else. Anything the request does not name stays closed.
fn wasi_builder(request: &Value, audit: Option<Arc<Audit>>, capture: Option<&Capture>) -> Result<WasiCtxBuilder, String> {
    let mut wasi = WasiCtxBuilder::new();
    let args: Vec<String> = strings(&request["args"]);
    wasi.args(&args);
    for pair in request["env"].as_array().into_iter().flatten() {
        if let (Some(name), Some(value)) = (pair[0].as_str(), pair[1].as_str()) {
            wasi.env(name, value);
        }
    }
    match capture {
        Some(capture) => {
            wasi.stdin(MemoryInputPipe::new(capture.stdin.clone()));
            wasi.stdout(capture.out.clone());
            wasi.stderr(capture.err.clone());
        }
        None => {
            if request["stdin"].as_bool().unwrap_or(false) {
                wasi.inherit_stdin();
            }
            wasi.inherit_stdout().inherit_stderr();
        }
    }
    for mount in request["mounts"].as_array().into_iter().flatten() {
        let (Some(host), Some(guest)) = (mount["host"].as_str(), mount["guest"].as_str()) else {
            return Err("a mount needs a host and a guest path".into());
        };
        let perms = if mount["readonly"].as_bool().unwrap_or(false) { FsPerms::ReadOnly } else { FsPerms::ReadWrite };
        wasi.preopened_dir(host, guest, perms).map_err(|error| format!("cannot mount {host}: {error}"))?;
    }
    open_network(&mut wasi, &request["net"], audit)?;
    Ok(wasi)
}

fn strings(value: &Value) -> Vec<String> {
    value.as_array().into_iter().flatten().filter_map(|item| item.as_str().map(str::to_owned)).collect()
}

/// `"none"` (the default) leaves sockets and name lookup closed. `"all"`
/// opens outbound connections anywhere. A list opens exactly the listed
/// destinations; a host name in it is resolved once, here, before the guest
/// runs, so the guest cannot steer where a granted name points afterwards.
/// Listening is never opened by a network grant.
fn open_network(wasi: &mut WasiCtxBuilder, net: &Value, audit: Option<Arc<Audit>>) -> Result<(), String> {
    let rules: Vec<NetRule> = match net["mode"].as_str().unwrap_or("none") {
        "none" => return Ok(()),
        "all" => vec![NetRule { ip: None, port: None }],
        "list" => {
            let mut rules = Vec::new();
            for entry in net["allow"].as_array().into_iter().flatten() {
                rules.extend(net_rules(entry)?);
            }
            rules
        }
        other => return Err(format!("unknown network mode '{other}'")),
    };
    wasi.allow_tcp(true).allow_udp(true).allow_ip_name_lookup(true);
    wasi.socket_addr_check(move |addr, usage| {
        let outbound = matches!(usage, SocketAddrUse::TcpConnect | SocketAddrUse::UdpSend | SocketAddrUse::UdpReceive);
        // The implicit bind before a connect arrives as a bind to the
        // wildcard address on port 0; without it no connection can be made.
        let implicit_bind = matches!(usage, SocketAddrUse::TcpBind | SocketAddrUse::UdpBind)
            && addr.ip().is_unspecified()
            && addr.port() == 0;
        let admitted = implicit_bind || (outbound && rules.iter().any(|rule| rule.admits(&addr)));
        if let (Some(audit), false) = (&audit, implicit_bind) {
            audit.socket(&addr, &format!("{usage:?}"), admitted);
        }
        Box::pin(async move { admitted })
    });
    Ok(())
}

fn net_rules(entry: &Value) -> Result<Vec<NetRule>, String> {
    let host = entry["host"].as_str().unwrap_or("*");
    let port = match entry["port"].as_u64() {
        Some(0) | None => None,
        Some(port) => Some(u16::try_from(port).map_err(|_| format!("port {port} is out of range"))?),
    };
    if host == "*" {
        return Ok(vec![NetRule { ip: None, port }]);
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![NetRule { ip: Some(ip), port }]);
    }
    let resolved = (host, port.unwrap_or(0)).to_socket_addrs().map_err(|error| format!("cannot resolve {host}: {error}"))?;
    Ok(resolved.map(|addr| NetRule { ip: Some(addr.ip()), port }).collect())
}

fn store_limits(limits: &Limits) -> StoreLimits {
    let builder = StoreLimitsBuilder::new();
    if limits.memory_bytes > 0 { builder.memory_size(limits.memory_bytes).build() } else { builder.build() }
}

/// How a run that reached its entry point ended.
type Ended = Result<Result<(), Error>, String>;

fn prepare<T: 'static>(store: &mut Store<T>, limits: &Limits) {
    if limits.fuel > 0 {
        let _ = store.set_fuel(limits.fuel);
    }
    if limits.timeout_ms > 0 {
        store.set_epoch_deadline(1);
    }
}

/// A core module: WASI preview 1, entered at `_start`.
fn run_module(engine: &Engine, bytes: &[u8], mut wasi: WasiCtxBuilder, limits: &Limits) -> Ended {
    let module = Module::from_binary(engine, bytes).map_err(|error| format!("not a runnable module: {error}"))?;
    let mut store = Store::new(engine, ModuleCtx { wasi: wasi.build_p1(), limits: store_limits(limits) });
    store.limiter(|ctx| &mut ctx.limits);
    prepare(&mut store, limits);
    let mut linker: Linker<ModuleCtx> = Linker::new(engine);
    wasmtime_wasi::p1::add_to_linker_async(&mut linker, |ctx| &mut ctx.wasi).map_err(|error| format!("linker setup failed: {error}"))?;
    wasmtime_wasi::runtime::in_tokio(async {
        let instance = linker.instantiate_async(&mut store, &module).await.map_err(|error| format!("cannot instantiate: {error}"))?;
        let start = instance
            .get_typed_func::<(), ()>(&mut store, "_start")
            .map_err(|_| "the module exports no `_start`; nautra runs WASI commands".to_string())?;
        Ok(start.call_async(&mut store, ()).await)
    })
}

/// A component: `wasi:cli/run`, through the WASI 0.3 interfaces when it
/// imports them and the 0.2 ones otherwise.
fn run_component(engine: &Engine, bytes: &[u8], ctx: ComponentCtx, limits: &Limits) -> Ended {
    let component = Component::from_binary(engine, bytes).map_err(|error| format!("not a runnable component: {error}"))?;
    let p3 = component.component_type().imports(engine).any(|(name, _)| name.starts_with("wasi:") && name.contains("@0.3"));
    let mut store = Store::new(engine, ctx);
    store.limiter(|ctx| &mut ctx.limits);
    prepare(&mut store, limits);
    let mut linker: ComponentLinker<ComponentCtx> = ComponentLinker::new(engine);
    let ran: Result<Result<Result<(), ()>, Error>, String> = if p3 {
        use wasmtime_wasi::p3::bindings::Command;
        wasmtime_wasi::p3::add_to_linker(&mut linker).map_err(|error| format!("linker setup failed: {error}"))?;
        wasmtime_wasi_http::p3::add_to_linker(&mut linker).map_err(|error| format!("linker setup failed: {error}"))?;
        wasmtime_wasi::runtime::in_tokio(async {
            let command = Command::instantiate_async(&mut store, &component, &linker).await.map_err(|error| not_a_command(&error))?;
            Ok(match store.run_concurrent(async |accessor| command.wasi_cli_run().call_run(accessor).await).await {
                Ok(inner) => inner,
                Err(error) => Err(error),
            })
        })
    } else {
        use wasmtime_wasi::p2::bindings::Command;
        wasmtime_wasi::p2::add_to_linker_async(&mut linker).map_err(|error| format!("linker setup failed: {error}"))?;
        wasmtime_wasi_http::p2::add_only_http_to_linker_async(&mut linker).map_err(|error| format!("linker setup failed: {error}"))?;
        wasmtime_wasi::runtime::in_tokio(async {
            let command = Command::instantiate_async(&mut store, &component, &linker).await.map_err(|error| not_a_command(&error))?;
            Ok(command.wasi_cli_run().call_run(&mut store).await)
        })
    };
    // `run` returning its own error is the component's exit 1, not a trap.
    Ok(match ran? {
        Ok(Ok(())) => Ok(()),
        Ok(Err(())) => Err(Error::new(I32Exit(1))),
        Err(error) => Err(error),
    })
}

fn not_a_command(error: &Error) -> String {
    format!("cannot instantiate: {error:#}")
}

fn exit_report(result: Result<(), Error>, limits: &Limits) -> Value {
    let Err(error) = result else { return json!({ "exit_code": 0 }) };
    // proc_exit arrives as an error but is an ordinary exit with its own code.
    if let Some(exit) = error.downcast_ref::<I32Exit>() {
        return json!({ "exit_code": exit.0 });
    }
    match error.downcast_ref::<Trap>() {
        Some(Trap::Interrupt) => json!({
            "exit_code": EXIT_TIMEOUT,
            "error": format!("stopped: the run passed its {} ms deadline", limits.timeout_ms),
        }),
        Some(Trap::OutOfFuel) => json!({
            "exit_code": EXIT_CEILING,
            "error": format!("stopped: the run used up its fuel ({})", limits.fuel),
        }),
        _ => json!({ "exit_code": EXIT_TRAPPED, "error": format!("trapped: {error:#}") }),
    }
}
