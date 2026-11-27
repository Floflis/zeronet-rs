//! ZirohNeth's supervised Iroh blob-transfer sidecar.
//!
//! The process owns one persistent Iroh endpoint and one filesystem-backed
//! iroh-blobs store. Python communicates through bounded JSON Lines on stdin
//! and stdout. Diagnostics are written only to stderr.

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::{anyhow, bail, Context, Result};
use iroh::{protocol::Router, Endpoint, NodeAddr, NodeId, RelayUrl, SecretKey};
use iroh_blobs::{
    net_protocol::Blobs,
    rpc::client::blobs::WrapOption,
    store::{ExportFormat, ExportMode},
    util::SetTagOption,
    Hash,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};

const PROTOCOL_VERSION: u32 = 1;
const MAX_REQUEST_BYTES: usize = 64 * 1024;

#[derive(Debug, Deserialize)]
struct Request {
    protocol_version: u32,
    request_id: Value,
    op: String,
    #[serde(default)]
    args: Value,
}

struct Bridge {
    data_dir: PathBuf,
    endpoint: Endpoint,
    blobs: Blobs<iroh_blobs::store::fs::Store>,
    _router: Router,
}

impl Bridge {
    async fn new(data_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&data_dir)
            .with_context(|| format!("creating bridge data directory {}", data_dir.display()))?;
        let data_dir = std::fs::canonicalize(&data_dir)?;
        let secret_key = load_or_create_endpoint_key(&data_dir)?;

        // The endpoint identity is stable across restarts. Its secret key is
        // private local state and is never returned through the JSONL API.
        let endpoint = Endpoint::builder()
            .secret_key(secret_key)
            .discovery_n0()
            .bind()
            .await
            .context("starting Iroh endpoint")?;

        let store_path = data_dir.join("blobs");
        std::fs::create_dir_all(&store_path)?;
        let blobs = Blobs::persistent(&store_path)
            .await
            .context("opening persistent Iroh blob store")?
            .build(&endpoint);
        let router = Router::builder(endpoint.clone())
            .accept(iroh_blobs::ALPN, blobs.clone())
            .spawn();

        eprintln!(
            "Iroh bridge ready: endpoint={}, data_dir={}",
            endpoint.node_id(),
            data_dir.display()
        );

        Ok(Self {
            data_dir,
            endpoint,
            blobs,
            _router: router,
        })
    }

    async fn handle(&self, op: &str, args: &Value) -> Result<(Value, bool)> {
        match op {
            "health" => Ok((
                json!({
                    "service": "zirohneth-iroh-bridge",
                    "protocol_version": PROTOCOL_VERSION,
                    "endpoint_id": self.endpoint.node_id().to_string()
                }),
                false,
            )),
            "node_addr" => {
                let address = self.endpoint.node_addr().await?;
                Ok((node_addr_to_json(&address), false))
            }
            "blob.add" => {
                let supplied_path = required_string(args, "path")?;
                let path = safe_existing_file(&self.data_dir, supplied_path)?;
                let size = std::fs::metadata(&path)?.len();
                let client = self.blobs.client();
                let added = client
                    .add_from_path(
                        path,
                        false,
                        SetTagOption::Auto,
                        WrapOption::NoWrap,
                    )
                    .await?
                    .finish()
                    .await
                    .context("adding file to the Iroh blob store")?;
                let address = self.endpoint.node_addr().await?;
                Ok((
                    json!({
                        "hash": added.hash.to_string(),
                        "format": format!("{:?}", added.format).to_lowercase(),
                        "size": size,
                        "node_addr": node_addr_to_json(&address)
                    }),
                    false,
                ))
            }
            "blob.fetch" => {
                let hash_text = required_string(args, "hash")?;
                let hash = Hash::from_str(hash_text).context("invalid BLAKE3 blob hash")?;
                let address = node_addr_from_json(
                    args.get("node_addr")
                        .ok_or_else(|| anyhow!("missing node_addr"))?,
                )?;
                let destination_text = required_string(args, "destination")?;
                let destination = safe_destination(&self.data_dir, destination_text)?;

                let client = self.blobs.client();
                client
                    .download(hash, address)
                    .await
                    .context("requesting Iroh blob")?
                    .finish()
                    .await
                    .context("downloading and verifying Iroh blob")?;
                client
                    .export(
                        hash,
                        destination.clone(),
                        ExportFormat::Blob,
                        ExportMode::Copy,
                    )
                    .await?
                    .finish()
                    .await
                    .context("exporting verified Iroh blob")?;

                let size = std::fs::metadata(&destination)?.len();
                Ok((
                    json!({
                        "hash": hash.to_string(),
                        "size": size,
                        "destination": destination.to_string_lossy()
                    }),
                    false,
                ))
            }
            "shutdown" => Ok((json!({"stopping": true}), true)),
            other => bail!("unsupported operation: {other}"),
        }
    }
}

fn required_string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| anyhow!("missing or invalid string field: {key}"))
}

fn node_addr_to_json(address: &NodeAddr) -> Value {
    let relay_url = address.relay_url().map(ToString::to_string);
    let direct_addresses: Vec<String> = address
        .direct_addresses()
        .map(ToString::to_string)
        .collect();

    json!({
        "node_id": address.node_id.to_string(),
        "relay_url": relay_url,
        "direct_addresses": direct_addresses
    })
}

fn node_addr_from_json(value: &Value) -> Result<NodeAddr> {
    let node_id_text = required_string(value, "node_id")?;
    let node_id = NodeId::from_str(node_id_text).context("invalid Iroh node_id")?;

    let relay_url = match value.get("relay_url") {
        Some(Value::String(text)) if !text.is_empty() => {
            Some(RelayUrl::from_str(text).context("invalid Iroh relay_url")?)
        }
        Some(Value::Null) | None => None,
        _ => bail!("relay_url must be a string or null"),
    };

    let direct_addresses = match value.get("direct_addresses") {
        Some(Value::Array(values)) => values
            .iter()
            .map(|item| {
                let text = item
                    .as_str()
                    .ok_or_else(|| anyhow!("direct address must be a string"))?;
                SocketAddr::from_str(text).context("invalid Iroh direct address")
            })
            .collect::<Result<Vec<_>>>()?,
        None => Vec::new(),
        _ => bail!("direct_addresses must be an array"),
    };

    Ok(NodeAddr::from_parts(
        node_id,
        relay_url,
        direct_addresses,
    ))
}

fn safe_existing_file(root: &Path, supplied: &str) -> Result<PathBuf> {
    let input = PathBuf::from(supplied);
    let candidate = if input.is_absolute() {
        input
    } else {
        root.join(input)
    };
    let canonical = std::fs::canonicalize(&candidate)
        .with_context(|| format!("resolving input file {}", candidate.display()))?;
    if !canonical.starts_with(root) {
        bail!("input path is outside the bridge data directory");
    }
    if !canonical.is_file() {
        bail!("input path is not a regular file");
    }
    Ok(canonical)
}

fn safe_destination(root: &Path, supplied: &str) -> Result<PathBuf> {
    let input = PathBuf::from(supplied);
    let candidate = if input.is_absolute() {
        input
    } else {
        root.join(input)
    };
    let parent = candidate
        .parent()
        .ok_or_else(|| anyhow!("destination has no parent directory"))?;
    std::fs::create_dir_all(parent)?;
    let canonical_parent = std::fs::canonicalize(parent)?;
    if !canonical_parent.starts_with(root) {
        bail!("destination path is outside the bridge data directory");
    }
    let name = candidate
        .file_name()
        .ok_or_else(|| anyhow!("destination has no filename"))?;
    let destination = canonical_parent.join(name);
    if destination.exists() {
        let canonical_existing = std::fs::canonicalize(&destination)?;
        if !canonical_existing.starts_with(root) || !canonical_existing.is_file() {
            bail!("destination is not a regular file inside the bridge data directory");
        }
    }
    Ok(destination)
}

fn load_or_create_endpoint_key(data_dir: &Path) -> Result<SecretKey> {
    let key_path = data_dir.join("endpoint.key");
    if key_path.exists() {
        let bytes = std::fs::read(&key_path)
            .with_context(|| format!("reading {}", key_path.display()))?;
        let raw: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("endpoint.key must contain exactly 32 bytes"))?;
        return Ok(SecretKey::from_bytes(&raw));
    }

    let key = SecretKey::generate(rand::thread_rng());
    std::fs::write(&key_path, key.to_bytes())
        .with_context(|| format!("writing {}", key_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))?;
    }

    Ok(key)
}

fn parse_data_dir() -> Result<PathBuf> {
    let mut args = std::env::args().skip(1);
    let mut data_dir = PathBuf::from("./data");
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--data-dir" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow!("--data-dir requires a path"))?;
                data_dir = PathBuf::from(value);
            }
            "--help" | "-h" => {
                println!("zirohneth-iroh-bridge --data-dir PATH");
                std::process::exit(0);
            }
            other => bail!("unknown argument: {other}"),
        }
    }
    Ok(data_dir)
}

fn error_response(request_id: Value, message: impl ToString) -> Value {
    json!({
        "protocol_version": PROTOCOL_VERSION,
        "request_id": request_id,
        "ok": false,
        "error": message.to_string()
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let data_dir = parse_data_dir()?;
    let bridge = Bridge::new(data_dir).await?;

    let stdin = BufReader::new(io::stdin());
    let mut lines = stdin.lines();
    let mut stdout = io::BufWriter::new(io::stdout());

    while let Some(line) = lines.next_line().await? {
        if line.len() > MAX_REQUEST_BYTES {
            let response = error_response(Value::Null, "request exceeds 64 KiB limit");
            stdout.write_all(serde_json::to_string(&response)?.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
            continue;
        }

        let parsed = serde_json::from_str::<Request>(&line);
        let request = match parsed {
            Ok(request) => request,
            Err(error) => {
                let response = error_response(Value::Null, format!("invalid JSON request: {error}"));
                stdout.write_all(serde_json::to_string(&response)?.as_bytes()).await?;
                stdout.write_all(b"\n").await?;
                stdout.flush().await?;
                continue;
            }
        };

        let request_id = request.request_id.clone();
        let response = if request.protocol_version != PROTOCOL_VERSION {
            error_response(
                request_id.clone(),
                format!(
                    "unsupported protocol_version {}; expected {}",
                    request.protocol_version, PROTOCOL_VERSION
                ),
            )
        } else {
            match bridge.handle(&request.op, &request.args).await {
                Ok((result, _)) => json!({
                    "protocol_version": PROTOCOL_VERSION,
                    "request_id": request_id,
                    "ok": true,
                    "result": result
                }),
                Err(error) => error_response(request_id.clone(), format!("{error:#}")),
            }
        };

        let should_stop = request.protocol_version == PROTOCOL_VERSION && request.op == "shutdown";
        stdout.write_all(serde_json::to_string(&response)?.as_bytes()).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
        if should_stop {
            break;
        }
    }

    bridge._router.shutdown().await?;
    bridge.endpoint.close().await;
    eprintln!("Iroh bridge stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::thread_rng;

    #[test]
    fn node_address_json_round_trip() {
        let node_id = SecretKey::generate(thread_rng()).public();
        let address = NodeAddr::from_parts(node_id, None, Vec::<SocketAddr>::new());
        let encoded = node_addr_to_json(&address);
        let decoded = node_addr_from_json(&encoded).unwrap();
        assert_eq!(decoded.node_id, address.node_id);
        assert_eq!(
            decoded.direct_addresses().count(),
            address.direct_addresses().count()
        );
    }

    #[test]
    fn bridge_paths_reject_parent_traversal() {
        let temp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        assert!(safe_destination(&root, "../outside.bin").is_err());
    }
}
