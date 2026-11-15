// SPDX-License-Identifier: Apache-2.0
//! Serving an HTTP component on a published address.
//!
//! A Wasm HTTP service has no port of its own: it exports a handler, and the
//! host decides where requests come from. `-p` names that place. The guest
//! gets the same grants as any run — nothing by default — and each instance
//! starts from a fresh view built from the request.

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use hyper::body::{Bytes, Incoming};
use serde_json::{json, Value};
use wasmtime::component::{Component, GuestTaskId, Linker};
use wasmtime::{Engine, Store, StoreContextMut};
use wasmtime_wasi_http::handler::{
    HandlerState, Instance, ProxyHandler, ProxyPre, ShouldAccept, WorkerExpiration, WorkerState, WorkerStatus,
};
use wasmtime_wasi_http::{WasiBody, WasiHttpCtxView, WasiHttpView};

use super::outbound::Audit;
use super::tools::Tools;
use super::{engine_config, inspect, refused, wasi_builder, ComponentCtx, Limits};

/// How often a guest that is computing yields to the host, so that a request
/// past its deadline can be abandoned even when the guest never waits.
const YIELD_EVERY: Duration = Duration::from_millis(10);

/// How long past a request deadline an instance may live before it is dropped
/// from outside its own event loop.
const GRACE: Duration = Duration::from_millis(250);

/// Serves until interrupted. Returns `{"exit_code": 0}` after a clean stop,
/// or an error document when the service could not be started.
pub fn nt_serve(request_json: &str) -> String {
    let outcome = match serde_json::from_str::<Value>(request_json) {
        Ok(request) => serve(request).unwrap_or_else(refused),
        Err(error) => refused(format!("malformed serve request: {error}")),
    };
    outcome.to_string()
}

fn serve(request: Value) -> Result<Value, String> {
    let listen = request["listen"].as_str().ok_or("serve request has no address")?;
    let address: SocketAddr = listen.parse().map_err(|_| format!("'{listen}' is not an address to listen on"))?;
    // No workload: what is published is the composition's tools themselves.
    let path = match request["path"].as_str() {
        Some(path) if !path.is_empty() => path.to_owned(),
        _ => return publish_tools(address, &request),
    };
    // A workload that ends, published: run once for each request.
    if request["invoke"] == json!(true) {
        return publish_agent(address, &request);
    }
    let bytes = std::fs::read(&path).map_err(|error| format!("cannot read {path}: {error}"))?;
    if !inspect::is_component(&bytes) {
        return Err("-p publishes an HTTP service, and this is a core module with no HTTP handler".into());
    }
    let limits = Limits::of(&request);
    let engine = Engine::new(&engine_config(&limits)).map_err(|error| format!("engine setup failed: {error}"))?;
    let component = Component::from_binary(&engine, &bytes).map_err(|error| format!("not a runnable component: {error}"))?;
    let pre = handler_of(&engine, &component)?;
    if limits.timeout_ms > 0 {
        let ticking = engine.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(YIELD_EVERY);
            ticking.increment_epoch();
        });
    }
    let audit = Audit::of(&request)?;
    if request["journal"]["path"].as_str().is_some_and(|path| !path.is_empty()) {
        return Err("a published service takes requests from outside, which a record cannot replay: --record is for a run that ends".into());
    }
    let tools = Tools::of(&request, audit.clone(), None)?;
    let serving = Serving { engine, pre, request, limits, audit, tools };
    wasmtime_wasi::runtime::in_tokio(listen_and_serve(address, serving))?;
    Ok(json!({ "exit_code": 0 }))
}

/// Links the component against WASI and `wasi:http`, and finds which handler
/// it exports: `wasi:http/handler` (0.3) or `wasi:http/incoming-handler` (0.2).
fn handler_of(engine: &Engine, component: &Component) -> Result<ProxyPre<ComponentCtx>, String> {
    let setup = |error: wasmtime::Error| format!("linker setup failed: {error}");
    let mut linker: Linker<ComponentCtx> = Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker).map_err(setup)?;
    wasmtime_wasi::p3::add_to_linker(&mut linker).map_err(setup)?;
    wasmtime_wasi_http::p2::add_only_http_to_linker_async(&mut linker).map_err(setup)?;
    wasmtime_wasi_http::p3::add_to_linker(&mut linker).map_err(setup)?;
    let exports: Vec<String> = component.component_type().exports(engine).map(|(name, _)| name.to_owned()).collect();
    let exported = |prefix: &str| exports.iter().any(|name| name.starts_with(prefix));
    let p3 = exported("wasi:http/handler@0.3");
    if !p3 && !exported("wasi:http/incoming-handler@0.2") {
        return Err("-p publishes an HTTP service, and this component exports no wasi:http handler".into());
    }
    let pre = linker.instantiate_pre(component).map_err(|error| format!("cannot instantiate: {error:#}"))?;
    let cannot = |error: wasmtime::Error| format!("cannot instantiate: {error:#}");
    if p3 {
        wasmtime_wasi_http::p3::bindings::ServicePre::new(pre).map(ProxyPre::P3).map_err(cannot)
    } else {
        wasmtime_wasi_http::p2::bindings::ProxyPre::new(pre).map(ProxyPre::P2).map_err(cannot)
    }
}

/// What every instance of the service is made from.
struct Serving {
    engine: Engine,
    pre: ProxyPre<ComponentCtx>,
    request: Value,
    limits: Limits,
    audit: Option<Arc<Audit>>,
    tools: Option<Arc<Tools>>,
}

fn http_view(ctx: &mut ComponentCtx) -> WasiHttpCtxView<'_> {
    ctx.http()
}

impl HandlerState for Serving {
    type StoreData = ComponentCtx;
    type WorkerExpiration = Expiry;
    type WorkerState = Worker;

    fn instantiate(&self) -> impl Future<Output = wasmtime::Result<Instance<ComponentCtx, Expiry, Worker>>> + Send {
        async move {
            let wasi = wasi_builder(&self.request, self.audit.clone(), None).map_err(wasmtime::Error::msg)?;
            let mut store = Store::new(&self.engine, ComponentCtx::new(wasi, &self.request, &self.limits, self.audit.clone(), self.tools.clone(), None));
            store.limiter(|ctx| &mut ctx.limits);
            if self.limits.fuel > 0 {
                store.set_fuel(self.limits.fuel)?;
            }
            let deadline = (self.limits.timeout_ms > 0).then(|| Duration::from_millis(self.limits.timeout_ms));
            if deadline.is_some() {
                store.epoch_deadline_async_yield_and_update(1);
            }
            let proxy = self.pre.instantiate_async(&mut store).await?;
            // A 0.3 handler is async and one instance takes requests side by
            // side; a 0.2 handler is given an instance of its own per request.
            let shared = matches!(self.pre, ProxyPre::P3(_));
            Ok(Instance {
                store,
                proxy,
                view: http_view,
                expiration: Expiry { deadline, timer: None },
                state: Worker { shared, deadline },
            })
        }
    }
}

struct Worker {
    shared: bool,
    deadline: Option<Duration>,
}

impl WorkerState for Worker {
    type StoreData = ComponentCtx;
    type RequestData = ();

    fn should_accept_request(&self, _concurrent: usize, total: usize) -> ShouldAccept {
        if self.shared || total == 0 { ShouldAccept::Yes } else { ShouldAccept::Never }
    }

    /// The returned future ending is the request's deadline passing.
    fn on_request_start(
        &self,
        _store: StoreContextMut<'_, ComponentCtx>,
        _data: (),
        _task: GuestTaskId,
    ) -> Pin<Box<dyn Future<Output = ()> + 'static + Send + Sync>> {
        match self.deadline {
            Some(limit) => Box::pin(tokio::time::sleep(limit)),
            None => Box::pin(std::future::pending()),
        }
    }

    fn drop(&self, _store: Store<ComponentCtx>, result: wasmtime::Result<()>) {
        if let Err(error) = result {
            eprintln!("nautra: an instance ended: {error:#}");
        }
    }
}

/// Drops an instance whose requests are all past their deadline, from outside
/// the instance's own event loop. Without a deadline an instance is kept.
struct Expiry {
    deadline: Option<Duration>,
    timer: Option<Pin<Box<tokio::time::Sleep>>>,
}

impl WorkerExpiration for Expiry {
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>, status: WorkerStatus, start: Instant) -> Poll<()> {
        let this = self.get_mut();
        match (status, this.deadline) {
            (WorkerStatus::Requests, Some(limit)) => {
                let timer = this.timer.insert(Box::pin(tokio::time::sleep_until((start + limit + GRACE).into())));
                timer.as_mut().poll(cx)
            }
            _ => Poll::Pending,
        }
    }
}

async fn listen_and_serve(address: SocketAddr, serving: Serving) -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind(address).await.map_err(|error| format!("cannot listen on {address}: {error}"))?;
    let bound = listener.local_addr().map_err(|error| error.to_string())?;
    eprintln!("nautra: serving on http://{bound}");
    let handler = ProxyHandler::new(serving);
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).map_err(|error| error.to_string())?;
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else { continue };
                let handler = handler.clone();
                tokio::spawn(async move {
                    let service = hyper::service::service_fn(move |request| respond(handler.clone(), request, bound));
                    let connection = hyper::server::conn::http1::Builder::new().serve_connection(hyper_util::rt::TokioIo::new(stream), service);
                    let _ = connection.await;
                });
            }
            _ = tokio::signal::ctrl_c() => return Ok(()),
            _ = terminate.recv() => return Ok(()),
        }
    }
}

/// One request through the guest. A request line carries only a path, and
/// `wasi:http` wants a whole URL, so the scheme and the authority are filled
/// in from the `Host` header or, without one, from the listening address.
async fn respond(handler: ProxyHandler<Serving>, request: hyper::Request<Incoming>, bound: SocketAddr) -> Result<hyper::Response<WasiBody>, std::convert::Infallible> {
    use http_body_util::BodyExt;
    let (mut parts, body) = request.into_parts();
    if parts.uri.scheme().is_none() {
        let authority = parts.headers.get(http::header::HOST).and_then(|host| host.to_str().ok()).map(str::to_owned).unwrap_or_else(|| bound.to_string());
        let target = parts.uri.path_and_query().map(|target| target.as_str()).unwrap_or("/").to_owned();
        if let Ok(uri) = http::Uri::builder().scheme("http").authority(authority).path_and_query(target).build() {
            parts.uri = uri;
        }
    }
    let body = body.map_err(wasmtime_wasi_http::Error::from).boxed_unsync();
    match handler.handle((), http::Request::from_parts(parts, body)).await {
        Ok(response) => Ok(response),
        Err(error) => {
            eprintln!("nautra: a request failed: {error:#}");
            let empty = http_body_util::Empty::<Bytes>::new().map_err(|never| match never {}).boxed_unsync();
            let mut response = hyper::Response::new(empty);
            *response.status_mut() = http::StatusCode::INTERNAL_SERVER_ERROR;
            Ok(response)
        }
    }
}

/// What is published when no HTTP component is: a composition's tools, or a
/// workload that ends, run once for each request.
enum Published {
    Tools(Arc<Tools>),
    Agent(Value),
}

/// A composition's tools published as an MCP server: the caller is whoever
/// connects, where in a run it is the workload. Each call is the same fresh,
/// captured run under the tool's own grants, checked and recorded the same
/// way. Requests are JSON-RPC over `POST /mcp`, answered whole.
fn publish_tools(address: SocketAddr, request: &Value) -> Result<Value, String> {
    let audit = Audit::of(request)?;
    let tools = Tools::of(request, audit, None)?.ok_or("there are no tools to publish")?;
    publish(address, request, Published::Tools(tools), "tools published at", "/mcp")
}

/// A workload that ends, published: each POST is one whole run of it — its
/// tools and checks with it — given the request body on stdin, and answered
/// with what it wrote. Nothing is kept from one request to the next.
fn publish_agent(address: SocketAddr, request: &Value) -> Result<Value, String> {
    // A schema that is not one, or an audit file that cannot be opened, is
    // said now rather than on the first request.
    Tools::of(request, Audit::of(request)?, None)?;
    let mut each = request.clone();
    if let Some(fields) = each.as_object_mut() {
        fields.remove("invoke");
        fields.remove("token");
        fields.insert("listen".into(), json!(""));
    }
    publish(address, request, Published::Agent(each), "each POST runs the workload:", "/invocations")
}

fn publish(address: SocketAddr, request: &Value, what: Published, says: &'static str, path: &'static str) -> Result<Value, String> {
    let token = request["token"].as_str().filter(|token| !token.is_empty()).map(str::to_owned);
    wasmtime_wasi::runtime::in_tokio(listen_for(address, Arc::new(what), Arc::new(token), says, path))?;
    Ok(json!({ "exit_code": 0 }))
}

async fn listen_for(address: SocketAddr, what: Arc<Published>, token: Arc<Option<String>>, says: &str, path: &str) -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind(address).await.map_err(|error| format!("cannot listen on {address}: {error}"))?;
    let bound = listener.local_addr().map_err(|error| error.to_string())?;
    eprintln!("nautra: {says} http://{bound}{path}");
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).map_err(|error| error.to_string())?;
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else { continue };
                let (what, token) = (what.clone(), token.clone());
                tokio::spawn(async move {
                    let service = hyper::service::service_fn(move |request| answer(what.clone(), token.clone(), request));
                    let connection = hyper::server::conn::http1::Builder::new().serve_connection(hyper_util::rt::TokioIo::new(stream), service);
                    let _ = connection.await;
                });
            }
            _ = tokio::signal::ctrl_c() => return Ok(()),
            _ = terminate.recv() => return Ok(()),
        }
    }
}

/// Whether a request presents the token, compared without stopping at the
/// first byte that differs.
fn presents(headers: &http::HeaderMap, token: &str) -> bool {
    let given = headers.get(http::header::AUTHORIZATION).and_then(|value| value.to_str().ok()).and_then(|value| value.strip_prefix("Bearer ")).unwrap_or("");
    given.len() == token.len() && given.bytes().zip(token.bytes()).fold(0u8, |differs, (a, b)| differs | (a ^ b)) == 0
}

type Reply = hyper::Response<http_body_util::Full<Bytes>>;

fn reply_with(status: http::StatusCode, kind: &'static str, body: Vec<u8>) -> Reply {
    let mut response = hyper::Response::new(http_body_util::Full::new(Bytes::from(body)));
    *response.status_mut() = status;
    response.headers_mut().insert(http::header::CONTENT_TYPE, http::HeaderValue::from_static(kind));
    response
}

async fn answer(what: Arc<Published>, token: Arc<Option<String>>, request: hyper::Request<Incoming>) -> Result<Reply, std::convert::Infallible> {
    use http_body_util::BodyExt;
    let plain = |status: http::StatusCode, text: &str| reply_with(status, "text/plain; charset=utf-8", format!("{text}\n").into_bytes());
    // Whether this process is up, for whatever started it. It says nothing
    // about the tools or the workload and runs nothing, so it asks no token.
    if request.method() == http::Method::GET && request.uri().path() == "/ping" {
        return Ok(reply_with(http::StatusCode::OK, "application/json", br#"{"status":"Healthy"}"#.to_vec()));
    }
    if matches!(*what, Published::Tools(_)) && !matches!(request.uri().path(), "/mcp" | "/") {
        return Ok(plain(http::StatusCode::NOT_FOUND, "the tools are at /mcp"));
    }
    if request.method() != http::Method::POST {
        let mut response = plain(http::StatusCode::METHOD_NOT_ALLOWED, "requests are POSTed here");
        response.headers_mut().insert(http::header::ALLOW, http::HeaderValue::from_static("POST"));
        return Ok(response);
    }
    if let Some(token) = token.as_deref() {
        if !presents(request.headers(), token) {
            let mut response = plain(http::StatusCode::UNAUTHORIZED, "a bearer token is required");
            response.headers_mut().insert(http::header::WWW_AUTHENTICATE, http::HeaderValue::from_static("Bearer"));
            return Ok(response);
        }
    }
    let body = match http_body_util::Limited::new(request.into_body(), super::tools::MAX_BYTES).collect().await {
        Ok(body) => body.to_bytes(),
        Err(_) => return Ok(plain(http::StatusCode::PAYLOAD_TOO_LARGE, "the request is too large")),
    };
    Ok(match &*what {
        Published::Tools(tools) => {
            let answer = tools.answer(&body).await;
            reply_with(http::StatusCode::from_u16(answer.status).unwrap_or(http::StatusCode::OK), "application/json", answer.body)
        }
        Published::Agent(each) => {
            let outcome = super::tools::run_tool(each.clone(), body.to_vec()).await;
            if outcome.exit_code == 0 {
                // What the workload wrote is the answer, as it is. It is
                // called JSON when it is.
                let kind = if serde_json::from_str::<Value>(&outcome.stdout).is_ok() { "application/json" } else { "text/plain; charset=utf-8" };
                reply_with(http::StatusCode::OK, kind, outcome.stdout.into_bytes())
            } else {
                let said = json!({ "error": super::tools::said(&outcome), "exit_code": outcome.exit_code });
                reply_with(http::StatusCode::INTERNAL_SERVER_ERROR, "application/json", said.to_string().into_bytes())
            }
        }
    })
}
