// SPDX-License-Identifier: Apache-2.0
//! One HTTP request whose body is a file, sent byte for byte.
//!
//! This exists for a single reason: Almide 0.70's http client carries a
//! request body as text, so a binary body (a Wasm blob being pushed) does not
//! survive it. Everything else about talking to a registry — the protocol,
//! authentication, verification — stays in `src/registry.almd`. Remove this
//! when the stdlib client can send bytes (almide/almide#4744).

use std::time::Duration;

use serde_json::{json, Value};

/// Request: `{"method", "url", "headers": [[name, value]…], "path"}`.
/// Reply: `{"status", "headers": [[name, value]…], "body"}` for any complete
/// response, or `{"error"}` for a transport failure. Redirects are not
/// followed: the caller decides where its credentials may go.
pub fn nt_http_upload(request_json: &str) -> String {
    let reply = match serde_json::from_str::<Value>(request_json) {
        Ok(request) => upload(&request).unwrap_or_else(|reason| json!({ "error": reason })),
        Err(error) => json!({ "error": format!("malformed upload request: {error}") }),
    };
    reply.to_string()
}

fn upload(request: &Value) -> Result<Value, String> {
    let method = request["method"].as_str().ok_or("upload request has no method")?;
    let url = request["url"].as_str().ok_or("upload request has no url")?;
    let path = request["path"].as_str().ok_or("upload request has no path")?;
    let body = std::fs::read(path).map_err(|error| format!("cannot read {path}: {error}"))?;
    let agent = ureq::AgentBuilder::new().redirects(0).timeout_connect(Duration::from_secs(30)).build();
    let mut call = agent.request(method, url);
    for pair in request["headers"].as_array().into_iter().flatten() {
        if let (Some(name), Some(value)) = (pair[0].as_str(), pair[1].as_str()) {
            call = call.set(name, value);
        }
    }
    let response = match call.send_bytes(&body) {
        Ok(response) => response,
        // A 4xx or 5xx is a complete response; the caller reads its status.
        Err(ureq::Error::Status(_, response)) => response,
        Err(error) => return Err(error.to_string()),
    };
    let status = response.status();
    let headers: Vec<Value> = response
        .headers_names()
        .into_iter()
        .filter_map(|name| response.header(&name).map(|value| json!([name, value])))
        .collect();
    let text = response.into_string().unwrap_or_default();
    Ok(json!({ "status": status, "headers": headers, "body": text }))
}
