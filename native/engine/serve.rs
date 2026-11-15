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
    let path = request["path"].as_str().ok_or("serve request has no path")?.to_owned();
    let listen = request["listen"].as_str().ok_or("serve request has no address")?;
    let address: SocketAddr = listen.parse().map_err(|_| format!("'{listen}' is not an address to listen on"))?;
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
