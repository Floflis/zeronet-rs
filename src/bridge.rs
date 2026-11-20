//! Versioned stdio bridge for embedding the native ZeroNet core in Python clients.
//!
//! The process speaks one JSON request and one JSON response per line. Keep
//! stdout reserved for this protocol; diagnostics belong on stderr.

use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{json, Value};
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::{core::error::Error, discovery};

const PROTOCOL_NAME: &str = "zeronet-bridge";
const PROTOCOL_VERSION: u64 = 1;
const DISCOVERY_PORT: u16 = 1544;

// LAN discovery is configured once per bridge process. Site lists can then
// change without changing the peer identity or advertised fileserver port.
static LAN_CONFIGURED: AtomicBool = AtomicBool::new(false);

fn success_response(id: Value, result: Value) -> Value {
    json!({
        "protocol": PROTOCOL_NAME,
        "version": PROTOCOL_VERSION,
        "id": id,
        "ok": true,
        "result": result
    })
}

fn error_response(id: Value, code: &str, message: impl Into<String>) -> Value {
    json!({
        "protocol": PROTOCOL_NAME,
        "version": PROTOCOL_VERSION,
        "id": id,
        "ok": false,
        "error": {
            "code": code,
            "message": message.into()
        }
    })
}

fn required_string<'a>(params: &'a Value, key: &str) -> Result<&'a str, (String, String)> {
    params
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            (
                "invalid_params".into(),
                format!("params.{key} must be a non-empty string"),
            )
        })
}

fn required_u64(params: &Value, key: &str) -> Result<u64, (String, String)> {
    params
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            (
                "invalid_params".into(),
                format!("params.{key} must be an unsigned integer"),
            )
        })
}

fn required_string_array(
    params: &Value,
    key: &str,
) -> Result<Vec<String>, (String, String)> {
    let values = params
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| {
            (
                "invalid_params".into(),
                format!("params.{key} must be an array of strings"),
            )
        })?;

    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| {
                    (
                        "invalid_params".into(),
                        format!("params.{key}[{index}] must be a string"),
                    )
                })
        })
        .collect()
}

async fn dispatch(
    method: &str,
    params: &Value,
) -> Result<Value, (String, String)> {
    match method {
        "system.ping" => Ok(json!({
            "name": "zeronet-rs",
            "bridge_protocol": PROTOCOL_VERSION
        })),

        // Configure the native LAN service with the identity and listening
        // port owned by the Python ZeroNet-compatible client.
        "discovery.lan.configure" => {
            if LAN_CONFIGURED.load(Ordering::Acquire) {
                return Err((
                    "already_configured".into(),
                    "LAN identity is immutable for this bridge process; use discovery.lan.set_local_sites to update the site list".into(),
                ));
            }

            let peer_id = required_string(params, "peer_id")?.to_owned();
            let fileserver_port = required_u64(params, "fileserver_port")?;
            let fileserver_port = u16::try_from(fileserver_port).map_err(|_| {
                (
                    "invalid_params".into(),
                    "params.fileserver_port must fit in an unsigned 16-bit integer".into(),
                )
            })?;
            if fileserver_port <= 1 {
                return Err((
                    "invalid_params".into(),
                    "params.fileserver_port must be greater than 1".into(),
                ));
            }

            let revision = required_u64(params, "revision")?;
            let local_sites = required_string_array(params, "local_sites")?;

            discovery::lan::start(
                local_sites.clone(),
                peer_id,
                fileserver_port,
                revision,
            )
            .await
            .map_err(|error| ("lan_start_failed".into(), error.to_string()))?;

            LAN_CONFIGURED.store(true, Ordering::Release);

            Ok(json!({
                "configured": true,
                "broadcast_port": DISCOVERY_PORT,
                "local_sites": local_sites.len()
            }))
        }

        "discovery.lan.set_local_sites" => {
            if !LAN_CONFIGURED.load(Ordering::Acquire) {
                return Err((
                    "not_configured".into(),
                    "call discovery.lan.configure before updating the local site list".into(),
                ));
            }

            let local_sites = required_string_array(params, "local_sites")?;
            let count = local_sites.len();
            discovery::lan::set_local_sites(local_sites);

            Ok(json!({ "updated": count }))
        }

        "discovery.lan.discover" => {
            if !LAN_CONFIGURED.load(Ordering::Acquire) {
                return Err((
                    "not_configured".into(),
                    "call discovery.lan.configure before discovering peers".into(),
                ));
            }

            let site = required_string(params, "site")?;
            let mut peers = discovery::lan::discover(site)
                .await
                .into_iter()
                .map(|peer| peer.address().to_string())
                .collect::<Vec<_>>();
            peers.sort();

            Ok(json!({
                "site": site,
                "count": peers.len(),
                "peers": peers
            }))
        }

        _ => Err((
            "unknown_method".into(),
            format!("unsupported bridge method: {method}"),
        )),
    }
}

async fn handle_line(line: &str) -> Value {
    let request: Value = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(error) => {
            return error_response(
                Value::Null,
                "invalid_json",
                format!("request is not valid JSON: {error}"),
            );
        }
    };

    let id = request.get("id").cloned().unwrap_or(Value::Null);
    if !matches!(id, Value::String(_) | Value::Number(_)) {
        return error_response(
            id,
            "invalid_request",
            "request.id must be a string or number",
        );
    }

    if request.get("protocol").and_then(Value::as_str) != Some(PROTOCOL_NAME) {
        return error_response(
            id,
            "invalid_protocol",
            format!("request.protocol must be {PROTOCOL_NAME}"),
        );
    }

    if request.get("version").and_then(Value::as_u64) != Some(PROTOCOL_VERSION) {
        return error_response(
            id,
            "unsupported_version",
            format!("only bridge protocol version {PROTOCOL_VERSION} is supported"),
        );
    }

    let method = match request.get("method").and_then(Value::as_str) {
        Some(method) if !method.is_empty() => method,
        _ => {
            return error_response(
                id,
                "invalid_request",
                "request.method must be a non-empty string",
            );
        }
    };

    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    if !params.is_object() {
        return error_response(
            id,
            "invalid_request",
            "request.params must be a JSON object",
        );
    }

    match dispatch(method, &params).await {
        Ok(result) => success_response(id, result),
        Err((code, message)) => error_response(id, &code, message),
    }
}

/// Run the persistent JSON Lines bridge until the parent closes stdin.
pub async fn run() -> Result<(), Error> {
    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut stdout = io::stdout();
    let mut line = String::new();

    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line).await?;
        if bytes_read == 0 {
            break;
        }

        let response = handle_line(&line).await;
        let mut encoded = serde_json::to_vec(&response)?;
        encoded.push(b'\n');

        stdout.write_all(&encoded).await?;
        stdout.flush().await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ping_uses_the_versioned_response_envelope() {
        let response = handle_line(
            r#"{"protocol":"zeronet-bridge","version":1,"id":"probe-1","method":"system.ping","params":{}}"#,
        )
        .await;

        assert_eq!(response["protocol"], PROTOCOL_NAME);
        assert_eq!(response["version"], PROTOCOL_VERSION);
        assert_eq!(response["id"], "probe-1");
        assert_eq!(response["ok"], true);
        assert_eq!(response["result"]["name"], "zeronet-rs");
    }

    #[tokio::test]
    async fn rejects_an_unknown_protocol_version() {
        let response = handle_line(
            r#"{"protocol":"zeronet-bridge","version":99,"id":7,"method":"system.ping","params":{}}"#,
        )
        .await;

        assert_eq!(response["ok"], false);
        assert_eq!(response["error"]["code"], "unsupported_version");
        assert_eq!(response["id"], 7);
    }

    #[tokio::test]
    async fn rejects_malformed_json_without_terminating_the_bridge() {
        let response = handle_line("{not-json").await;

        assert_eq!(response["ok"], false);
        assert_eq!(response["error"]["code"], "invalid_json");
    }

    #[test]
    fn parses_site_lists_and_rejects_non_string_entries() {
        let valid = json!({ "local_sites": ["site-a", "site-b"] });
        assert_eq!(
            required_string_array(&valid, "local_sites").unwrap(),
            vec!["site-a".to_owned(), "site-b".to_owned()]
        );

        let invalid = json!({ "local_sites": ["site-a", 42] });
        assert!(required_string_array(&invalid, "local_sites").is_err());
    }

    #[tokio::test]
    async fn unknown_methods_return_a_structured_error() {
        let response = handle_line(
            r#"{"protocol":"zeronet-bridge","version":1,"id":"probe-2","method":"not.a.method","params":{}}"#,
        )
        .await;

        assert_eq!(response["ok"], false);
        assert_eq!(response["error"]["code"], "unknown_method");
    }
}
