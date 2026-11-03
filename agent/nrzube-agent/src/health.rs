//! Health checks: containerd, kubelet and the Kubernetes node object.

use anyhow::{Context, Result};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::client::conn::http2;
use hyper::{Request, StatusCode};
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::path::Path;
use tokio::net::UnixStream;

/// Default location of the containerd socket.
pub const CONTAINERD_SOCKET: &str = "/run/containerd/containerd.sock";
/// Default port of the kubelet health endpoint.
pub const KUBELET_HEALTH_PORT: u16 = 10250;

/// The result of a health check.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Health {
    /// containerd answers to the gRPC health check.
    pub containerd: bool,
    /// The kubelet answers to /healthz.
    pub kubelet: bool,
    /// The node object is Ready in the Kubernetes API.
    pub node_ready: bool,
}

impl Health {
    /// Whether the node is fully operational.
    pub fn is_healthy(&self) -> bool {
        self.containerd && self.kubelet
    }
}

/// Check containerd with the gRPC health checking protocol
/// (`/grpc.health.v1.Health/Check` over the unix socket).
pub async fn check_containerd(socket: &Path) -> bool {
    match grpc_health_check(socket).await {
        Ok(healthy) => healthy,
        Err(err) => {
            log::debug!("containerd health check failed: {err:#}");
            false
        }
    }
}

/// A minimal gRPC health check over a unix socket.
async fn grpc_health_check(socket: &Path) -> Result<bool> {
    let stream = UnixStream::connect(socket)
        .await
        .with_context(|| format!("connecting to {}", socket.display()))?;

    let (mut sender, connection) =
        http2::handshake(TokioExecutor::new(), TokioIo::new(stream)).await?;
    tokio::spawn(async move {
        let _ = connection.await;
    });

    // gRPC frame: compression flag (1 byte) + message length (4 bytes, big
    // endian). The request message is empty.
    let mut frame = Vec::with_capacity(5);
    frame.push(0);
    frame.extend_from_slice(&0u32.to_be_bytes());

    let request = Request::builder()
        .method("POST")
        .uri("http://localhost/grpc.health.v1.Health/Check")
        .header("content-type", "application/grpc")
        .header("te", "trailers")
        .header("user-agent", "nrzube-agent")
        .body(Full::new(Bytes::from(frame)))?;

    let response = sender.send_request(request).await?;
    if response.status() != StatusCode::OK {
        return Ok(false);
    }

    // A "trailers only" response carries the gRPC status in the headers.
    if let Some(status) = response.headers().get("grpc-status") {
        let value = status
            .to_str()
            .context("the grpc-status header is not valid ASCII")?;
        return Ok(value == "0");
    }

    let body = response.into_body().collect().await?.to_bytes();
    Ok(parse_health_response(&body))
}

/// Parse a `grpc.health.v1.HealthCheckResponse`.
///
/// The message is a protobuf: field 1 is the status (1 = SERVING). The body
/// is a gRPC frame: 1 byte of compression flag, 4 bytes of length, then the
/// message.
fn parse_health_response(body: &[u8]) -> bool {
    if body.len() < 5 {
        return false;
    }
    let message = &body[5..];
    // Field 1, wire type 0 (varint).
    if message.first() != Some(&0x08) {
        return false;
    }
    let mut value: u64 = 0;
    let mut shift = 0;
    for byte in message.iter().skip(1) {
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift > 63 {
            return false;
        }
    }
    // 1 = SERVING.
    value == 1
}

/// Build an HTTP client, optionally carrying the node identity.
///
/// The kubelet and the API server authenticate the client certificates: the
/// identity is therefore configured on the client itself.
pub fn build_client(identity_pem: Option<&str>) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .user_agent(concat!("nrzube-agent/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        // The kubelet serves its own certificate: the CA is not known.
        .danger_accept_invalid_certs(true);

    if let Some(pem) = identity_pem {
        let identity = reqwest::Identity::from_pem(pem.as_bytes())
            .context("building the client identity")?;
        builder = builder.identity(identity);
    }

    builder.build().context("building the HTTP client")
}

/// Check the kubelet health endpoint.
///
/// The kubelet authenticates the client certificates: the node identity is
/// used when it is available.
pub async fn check_kubelet(client: &reqwest::Client, port: u16) -> bool {
    let url = format!("https://127.0.0.1:{port}/healthz");
    match client.get(&url).timeout(std::time::Duration::from_secs(5)).send().await {
        Ok(response) => response.status().is_success(),
        Err(err) => {
            log::debug!("kubelet health check failed: {err}");
            false
        }
    }
}

/// Check that the node object is Ready in the Kubernetes API.
pub async fn check_node_ready(
    client: &reqwest::Client,
    api_server: &str,
    node_name: &str,
    token: Option<&str>,
) -> bool {
    let url = format!(
        "{}/api/v1/nodes/{}",
        api_server.trim_end_matches('/'),
        node_name
    );
    let mut request = client.get(&url).timeout(std::time::Duration::from_secs(10));

    if let Some(token) = token {
        request = request.bearer_auth(token);
    }

    let response = match request.send().await {
        Ok(response) => response,
        Err(err) => {
            log::debug!("node readiness check failed: {err}");
            return false;
        }
    };

    if !response.status().is_success() {
        return false;
    }

    let body: serde_json::Value = match response.json().await {
        Ok(body) => body,
        Err(err) => {
            log::debug!("cannot decode the node object: {err}");
            return false;
        }
    };

    body.get("status")
        .and_then(|status| status.get("conditions"))
        .and_then(|conditions| conditions.as_array())
        .map(|conditions| {
            conditions.iter().any(|condition| {
                condition.get("type").and_then(|t| t.as_str()) == Some("Ready")
                    && condition.get("status").and_then(|s| s.as_str()) == Some("True")
            })
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_serving_response() {
        // gRPC frame: compression flag, length (1), message (0x08 0x01).
        let body = [0u8, 0, 0, 0, 1, 0x08, 0x01];
        assert!(parse_health_response(&body));
    }

    #[test]
    fn rejects_a_not_serving_response() {
        let body = [0u8, 0, 0, 0, 1, 0x08, 0x02];
        assert!(!parse_health_response(&body));
    }

    #[test]
    fn rejects_a_truncated_response() {
        assert!(!parse_health_response(&[0, 0, 0]));
        assert!(!parse_health_response(&[]));
    }
}
