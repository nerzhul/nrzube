//! The HTTP server of the agent: `/healthz` and `/metrics` on
//! 127.0.0.1:12249.
//!
//! The port 12249 was chosen on purpose: 9100 is the conventional port of
//! node_exporter and is not used here.

use anyhow::{Context, Result};
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::service::service_fn;
use hyper::{Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::net::TcpListener;

use crate::metrics::Metrics;

/// The default listening address.
pub const DEFAULT_LISTEN: &str = "127.0.0.1:12249";

/// Serve the health and metrics endpoints until the process is stopped.
pub async fn serve(listener: TcpListener, metrics: Arc<Metrics>) -> Result<()> {
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(err) => {
                log::warn!("accept failed: {err}");
                continue;
            }
        };

        let metrics = metrics.clone();
        tokio::spawn(async move {
            let service = service_fn(move |request| {
                let metrics = metrics.clone();
                async move {
                    Ok::<_, Infallible>(handle(request.method(), request.uri().path(), &metrics))
                }
            });

            if let Err(err) = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await
            {
                log::debug!("connection from {peer} failed: {err}");
            }
        });
    }
}

fn handle(method: &hyper::Method, path: &str, metrics: &Metrics) -> Response<Full<Bytes>> {
    match (method, path) {
        (&hyper::Method::GET, "/healthz") => {
            let containerd = metrics.containerd_healthy.load(std::sync::atomic::Ordering::Relaxed);
            let kubelet = metrics.kubelet_healthy.load(std::sync::atomic::Ordering::Relaxed);
            let configured = metrics.configured.load(std::sync::atomic::Ordering::Relaxed);

            // Before the node is provisioned the kubelet is not expected to
            // run: the agent is alive, the node is not ready yet.
            let healthy = !configured || (containerd && kubelet);
            let body = format!("ok containerd={containerd} kubelet={kubelet}");

            let status = if healthy {
                StatusCode::OK
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            };
            response(status, body)
        }
        (&hyper::Method::GET, "/metrics") => {
            response(StatusCode::OK, metrics.render())
        }
        _ => response(StatusCode::NOT_FOUND, "not found".to_string()),
    }
}

fn response(status: StatusCode, body: String) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::from(body)));
    *response.status_mut() = status;
    response.headers_mut().insert(
        "content-type",
        hyper::header::HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
    );
    response
}

/// Bind the listening socket.
pub async fn bind(address: &str) -> Result<TcpListener> {
    TcpListener::bind(address)
        .await
        .with_context(|| format!("binding {address}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn healthz_is_ok_before_the_provisioning() {
        let metrics = Metrics::new();
        let response = handle(&hyper::Method::GET, "/healthz", &metrics);
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn healthz_is_unavailable_when_a_component_is_down() {
        let metrics = Metrics::new();
        metrics.configured.store(true, Ordering::Relaxed);
        metrics.containerd_healthy.store(true, Ordering::Relaxed);
        let response = handle(&hyper::Method::GET, "/healthz", &metrics);
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn metrics_are_exposed() {
        let metrics = Metrics::new();
        let response = handle(&hyper::Method::GET, "/metrics", &metrics);
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn unknown_paths_are_not_found() {
        let metrics = Metrics::new();
        let response = handle(&hyper::Method::GET, "/nope", &metrics);
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
