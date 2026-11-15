// SPDX-License-Identifier: Apache-2.0
//! What leaves a guest over `wasi:http`: which destinations it may reach,
//! which secrets are put on which requests, and the record of both.
//!
//! `wasi:http` hands the host a whole request rather than a socket, so this is
//! the one place every outbound HTTP request passes through, before any
//! connection exists.

use std::fs::{File, OpenOptions};
use std::future::Future;
use std::io::Write;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use http::HeaderValue;
use serde_json::{json, Value};
use wasmtime_wasi_http::{Error, RequestOptions, WasiBody, WasiHttpHooks};

use super::tools::{self, Tools};

/// The record of what a run did at its boundary: one JSON object per line,
/// appended. A line that cannot be written is dropped rather than failing
/// the run it describes.
pub(super) struct Audit(Mutex<File>);

impl Audit {
    /// Opens the file the request names, or `None` when it names none.
    pub(super) fn of(request: &Value) -> Result<Option<Arc<Audit>>, String> {
        let Some(path) = request["audit"].as_str().filter(|path| !path.is_empty()) else { return Ok(None) };
        let file = OpenOptions::new().create(true).append(true).open(path).map_err(|error| format!("cannot open {path}: {error}"))?;
        Ok(Some(Arc::new(Audit(Mutex::new(file)))))
    }

    pub(super) fn record(&self, mut record: Value) {
        record["time"] = json!(now_utc());
        if let Ok(mut file) = self.0.lock() {
            let _ = writeln!(file, "{record}");
        }
    }

    /// One socket use the guest attempted and how it was decided.
    pub(super) fn socket(&self, addr: &SocketAddr, usage: &str, admitted: bool) {
        self.record(json!({
            "kind": "socket",
            "decision": if admitted { "allow" } else { "deny" },
            "use": usage,
            "address": addr.ip().to_string(),
            "port": addr.port(),
        }));
    }
}

/// The current time as `YYYY-MM-DDTHH:MM:SSZ`.
fn now_utc() -> String {
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_secs()).unwrap_or(0) as i64;
    let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60)
}

/// A secret the guest may use without ever holding it. The guest is given
/// `placeholder` in place of the value; the value is put on a request only
/// when the request goes to one of `hosts`.
struct Secret {
    name: String,
    placeholder: String,
    value: String,
    hosts: Vec<(String, u16)>,
}

impl Secret {
    fn of(entry: &Value) -> Option<Secret> {
        let hosts = entry["hosts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|host| Some((host["host"].as_str()?.to_owned(), u16::try_from(host["port"].as_u64()?).ok()?)))
            .collect();
        Some(Secret {
            name: entry["name"].as_str()?.to_owned(),
            placeholder: entry["placeholder"].as_str().filter(|text| !text.is_empty())?.to_owned(),
            value: entry["value"].as_str()?.to_owned(),
            hosts,
        })
    }

    fn bound_to(&self, host: &str, port: u16) -> bool {
        self.hosts.iter().any(|(bound, bound_port)| bound.eq_ignore_ascii_case(host) && *bound_port == port)
    }
}

/// The outbound policy of one guest.
pub(super) struct Outbound {
    rules: Vec<(String, Option<u16>)>,
    secrets: Vec<Secret>,
    audit: Option<Arc<Audit>>,
    tools: Option<Arc<Tools>>,
}

/// What was decided about one request before it was sent.
struct Verdict {
    refusal: Option<String>,
    secrets: Vec<String>,
}

impl Outbound {
    pub(super) fn of(request: &Value, audit: Option<Arc<Audit>>, tools: Option<Arc<Tools>>) -> Outbound {
        let net = &request["net"];
        let rules = match net["mode"].as_str().unwrap_or("none") {
            "all" => vec![("*".to_owned(), None)],
            "list" => net["allow"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|entry| {
                    let port = entry["port"].as_u64().filter(|port| *port != 0).and_then(|port| u16::try_from(port).ok());
                    (entry["host"].as_str().unwrap_or("*").to_owned(), port)
                })
                .collect(),
            _ => Vec::new(),
        };
        let secrets: Vec<Secret> = request["secrets"].as_array().into_iter().flatten().filter_map(Secret::of).collect();
        // Binding a secret to a host is also the grant to send HTTP there:
        // a credential for a place the guest cannot reach would be no use.
        // It opens HTTP only; raw sockets to that host stay closed.
        let mut rules = rules;
        for secret in &secrets {
            rules.extend(secret.hosts.iter().map(|(host, port)| (host.clone(), Some(*port))));
        }
        Outbound { rules, secrets, audit, tools }
    }

    fn admits(&self, host: &str, port: u16) -> bool {
        self.rules.iter().any(|(granted, granted_port)| {
            (granted == "*" || granted.eq_ignore_ascii_case(host)) && granted_port.is_none_or(|granted| granted == port)
        })
    }

    /// Decides one request and, where a secret's placeholder travels to a
    /// destination the secret is bound to, puts the real value in its place.
    /// A placeholder going anywhere else — another host, an unencrypted
    /// connection, the URL itself — refuses the request: the guest is trying
    /// to send a secret somewhere it was not bound to go.
    fn decide(&self, request: &mut http::Request<WasiBody>, host: &str, port: u16, secure: bool) -> Verdict {
        let mut verdict = Verdict { refusal: None, secrets: Vec::new() };
        if !self.admits(host, port) {
            verdict.refusal = Some("the destination is not granted".into());
            return verdict;
        }
        let target = request.uri().to_string();
        for secret in &self.secrets {
            let carried = request.headers().values().any(|value| contains(value.as_bytes(), secret.placeholder.as_bytes()));
            if target.contains(&secret.placeholder) {
                verdict.refusal = Some(format!("secret {} may travel only in a header", secret.name));
            } else if carried && !secret.bound_to(host, port) {
                verdict.refusal = Some(format!("secret {} is not bound to this destination", secret.name));
            } else if carried && !secure && !is_loopback(host) {
                verdict.refusal = Some(format!("secret {} is not sent over an unencrypted connection", secret.name));
            } else if carried {
                for value in request.headers_mut().values_mut() {
                    if let Some(real) = substitute(value.as_bytes(), secret.placeholder.as_bytes(), secret.value.as_bytes()) {
                        if let Ok(mut replaced) = HeaderValue::from_bytes(&real) {
                            replaced.set_sensitive(true);
                            *value = replaced;
                        }
                    }
                }
                verdict.secrets.push(secret.name.clone());
            }
            if verdict.refusal.is_some() {
                return verdict;
            }
        }
        verdict
    }
}

fn is_loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost") || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|window| window == needle)
}

/// `haystack` with every `needle` replaced, or `None` when it holds none.
fn substitute(haystack: &[u8], needle: &[u8], with: &[u8]) -> Option<Vec<u8>> {
    if !contains(haystack, needle) {
        return None;
    }
    let mut out = Vec::with_capacity(haystack.len());
    let mut rest = haystack;
    while let Some(at) = rest.windows(needle.len()).position(|window| window == needle) {
        out.extend_from_slice(&rest[..at]);
        out.extend_from_slice(with);
        rest = &rest[at + needle.len()..];
    }
    out.extend_from_slice(rest);
    Some(out)
}

/// A request to the tools endpoint: answered here, never sent anywhere. A
/// guest that was given no tools is refused, as for any ungranted host.
async fn answer_locally(tools: Option<Arc<Tools>>, request: http::Request<WasiBody>) -> Sent {
    use http_body_util::BodyExt;
    let Some(tools) = tools else { return Err(Error::HttpRequestDenied) };
    let body = request.into_body().collect().await?.to_bytes();
    if body.len() > tools::MAX_BYTES {
        return Err(Error::HttpRequestDenied);
    }
    let answer = tools.answer(&body).await;
    let mut response = http::Response::new(http_body_util::Full::new(hyper::body::Bytes::from(answer.body)).map_err(|never| match never {}).boxed_unsync());
    *response.status_mut() = http::StatusCode::from_u16(answer.status).unwrap_or(http::StatusCode::OK);
    response.headers_mut().insert(http::header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    Ok((response, Box::new(async { Ok(()) }) as Box<dyn Future<Output = _> + Send>))
}

type Sent = Result<(http::Response<WasiBody>, Box<dyn Future<Output = Result<(), Error>> + Send>), Error>;

impl WasiHttpHooks for Outbound {
    fn send_request(
        &mut self,
        mut request: http::Request<WasiBody>,
        options: Option<RequestOptions>,
        _fut: Box<dyn Future<Output = Result<(), Error>> + Send>,
    ) -> Box<dyn Future<Output = Sent> + Send> {
        let secure = request.uri().scheme_str() == Some("https");
        let port = request.uri().port_u16().unwrap_or(if secure { 443 } else { 80 });
        let host = request.uri().host().unwrap_or("").trim_start_matches('[').trim_end_matches(']').to_owned();
        if host.eq_ignore_ascii_case(tools::HOST) {
            return Box::new(answer_locally(self.tools.clone(), request));
        }
        let verdict = self.decide(&mut request, &host, port, secure);
        // The record names where a request went and what it carried, never
        // the query string or a header value.
        let mut record = json!({
            "kind": "http",
            "decision": if verdict.refusal.is_none() { "allow" } else { "deny" },
            "method": request.method().as_str(),
            "scheme": if secure { "https" } else { "http" },
            "host": host,
            "port": port,
            "path": request.uri().path(),
            "secrets": verdict.secrets,
        });
        let audit = self.audit.clone();
        Box::new(async move {
            use http_body_util::BodyExt;
            if let Some(reason) = verdict.refusal {
                record["reason"] = json!(reason);
                if let Some(audit) = &audit {
                    audit.record(record);
                }
                return Err(Error::HttpRequestDenied);
            }
            let sent = wasmtime_wasi_http::default_send_request(request, options).await;
            match &sent {
                Ok((response, _)) => record["status"] = json!(response.status().as_u16()),
                Err(error) => record["error"] = json!(error.to_string()),
            }
            if let Some(audit) = &audit {
                audit.record(record);
            }
            let (response, io) = sent?;
            Ok((response.map(BodyExt::boxed_unsync), Box::new(io) as Box<dyn Future<Output = _> + Send>))
        })
    }
}
