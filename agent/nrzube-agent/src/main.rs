//! nrzube-agent entry point.

use anyhow::{Context, Result};
use log::LevelFilter;
use nrzube_agent::config::{dhcp_config_url, NodeConfig, Paths};
use nrzube_agent::cosign::TrustedKey;
use nrzube_agent::health;
use nrzube_agent::logging;
use nrzube_agent::metrics::Metrics;
use nrzube_agent::registry::Registry;
use nrzube_agent::server;
use nrzube_agent::util;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;

/// How often the health of the components is checked.
const HEALTH_INTERVAL: Duration = Duration::from_secs(15);
/// How often the registry is polled for updates.
const OTA_INTERVAL: Duration = Duration::from_secs(15 * 60);
/// How long the agent waits before retrying the node configuration.
const BOOTSTRAP_RETRY: Duration = Duration::from_secs(30);

#[derive(Debug)]
struct Args {
    listen: String,
    log_level: LevelFilter,
    /// Run a single OTA check and exit (used by the tests and by the CI).
    once: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            listen: server::DEFAULT_LISTEN.to_string(),
            log_level: LevelFilter::Info,
            once: false,
        }
    }
}

impl Args {
    fn parse() -> Self {
        let mut args = Args::default();
        let mut argv = std::env::args().skip(1);
        while let Some(argument) = argv.next() {
            match argument.as_str() {
                "--listen" => {
                    if let Some(value) = argv.next() {
                        args.listen = value;
                    }
                }
                "--log-level" => {
                    if let Some(value) = argv.next() {
                        args.log_level = match value.as_str() {
                            "debug" => LevelFilter::Debug,
                            "warn" => LevelFilter::Warn,
                            "error" => LevelFilter::Error,
                            _ => LevelFilter::Info,
                        };
                    }
                }
                "--once" => args.once = true,
                other => {
                    eprintln!("nrzube-agent: unknown argument {other}");
                }
            }
        }
        args
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    logging::init(args.log_level);

    let paths = Paths::default();
    logging::open_file(&paths.state_dir.join("agent.log"));

    let metrics = Arc::new(Metrics::new());
    let bootstrap_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("building the HTTP client")?;

    // The HTTP server starts immediately: the metrics are useful even before
    // the node is provisioned.
    let listener = server::bind(&args.listen).await?;
    log::info!("listening on {}", args.listen);
    let server_metrics = metrics.clone();
    tokio::spawn(async move {
        if let Err(err) = server::serve(listener, server_metrics).await {
            log::error!("the HTTP server stopped: {err:#}");
        }
    });

    // The bootstrap: the node configuration is retrieved once and persisted.
    let config = bootstrap(&paths, &bootstrap_client, &metrics).await;

    // The health of the components. The client carries the node identity so
    // that the kubelet and the API server accept the requests.
    let identity = client_identity(&config);
    let health_client = match health::build_client(identity.as_deref()) {
        Ok(client) => client,
        Err(err) => {
            log::warn!("cannot build an authenticated client: {err:#}");
            bootstrap_client.clone()
        }
    };
    let health_metrics = metrics.clone();
    let health_config = config.clone();
    tokio::spawn(async move {
        health_loop(&health_client, health_config, health_metrics).await;
    });

    // The OTA loop.
    let ota = build_ota(&paths, config, metrics.clone());
    if args.once {
        if let Some(ota) = ota {
            match ota.check_and_apply().await {
                Ok(outcome) => log::info!("OTA check finished: {outcome:?}"),
                Err(err) => log::error!("OTA check failed: {err:#}"),
            }
        }
        return Ok(());
    }

    let ota_metrics = metrics.clone();
    tokio::spawn(async move {
        ota_loop(ota, ota_metrics).await;
    });

    log::info!("nrzube-agent is running");
    let _ = signal::ctrl_c().await;
    log::info!("nrzube-agent is stopping");
    Ok(())
}

/// Retrieve (once) and persist the node configuration.
async fn bootstrap(
    paths: &Paths,
    client: &reqwest::Client,
    metrics: &Metrics,
) -> Option<NodeConfig> {
    loop {
        // The persisted configuration always wins: the node must boot and
        // rejoin the cluster without any network access.
        match NodeConfig::load(&paths.node_config) {
            Ok(Some(config)) => {
                if let Err(err) = config.write_derived_files(paths) {
                    log::error!("cannot write the derived files: {err:#}");
                }
                if let Err(err) = config.apply_hostname(paths) {
                    log::error!("cannot apply the node name: {err:#}");
                }
                metrics.configured.store(true, Ordering::Relaxed);
                mark_provisioned(paths);
                log::info!("the node configuration comes from the persisted state");
                return Some(config);
            }
            Ok(None) => {}
            Err(err) => {
                log::error!("the persisted node configuration is unusable: {err:#}");
            }
        }

        // First boot: the URL comes from the DHCP option 224.
        let Some(url) = dhcp_config_url(paths) else {
            log::info!("waiting for the node configuration URL (DHCP option 224)");
            tokio::time::sleep(BOOTSTRAP_RETRY).await;
            continue;
        };

        log::info!("fetching the node configuration from {url}");
        match fetch_config(client, &url).await {
            Ok(config) => {
                if let Err(err) = config.persist(&paths.node_config) {
                    log::error!("cannot persist the node configuration: {err:#}");
                    tokio::time::sleep(BOOTSTRAP_RETRY).await;
                    continue;
                }
                if let Err(err) = config.write_derived_files(paths) {
                    log::error!("cannot write the derived files: {err:#}");
                    tokio::time::sleep(BOOTSTRAP_RETRY).await;
                    continue;
                }
                if let Err(err) = config.apply_hostname(paths) {
                    log::error!("cannot apply the node name: {err:#}");
                    tokio::time::sleep(BOOTSTRAP_RETRY).await;
                    continue;
                }
                metrics.configured.store(true, Ordering::Relaxed);
                mark_provisioned(paths);
                log::info!("the node is provisioned");
                return Some(config);
            }
            Err(err) => {
                // Not fatal: the node retries until the server answers.
                log::error!("cannot fetch the node configuration: {err:#}");
                tokio::time::sleep(BOOTSTRAP_RETRY).await;
            }
        }
    }
}

fn mark_provisioned(paths: &Paths) {
    if let Err(err) = util::write_atomic_str(&paths.run_dir.join("provisioned"), "1") {
        log::error!("cannot write the provisioned marker: {err:#}");
    }
}

async fn fetch_config(client: &reqwest::Client, url: &str) -> Result<NodeConfig> {
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .with_context(|| format!("GET {url}"))?;
    let bytes = response.bytes().await.context("reading the response")?;
    NodeConfig::from_slice(&bytes)
}

/// The client identity (certificate + key) used to talk to the kubelet and to
/// the API server.
fn client_identity(config: &Option<NodeConfig>) -> Option<String> {
    let config = config.as_ref()?;
    let cert = config.client_cert.as_ref()?;
    let key = config.client_key.as_ref()?;
    Some(format!("{}\n{}", cert.trim(), key.trim()))
}

async fn health_loop(
    client: &reqwest::Client,
    config: Option<NodeConfig>,
    metrics: Arc<Metrics>,
) {
    loop {
        let containerd = health::check_containerd(Path::new(health::CONTAINERD_SOCKET)).await;
        metrics.containerd_healthy.store(containerd, Ordering::Relaxed);

        let kubelet = health::check_kubelet(client, health::KUBELET_HEALTH_PORT).await;
        metrics.kubelet_healthy.store(kubelet, Ordering::Relaxed);

        if let Some(config) = &config {
            let ready = health::check_node_ready(
                client,
                config.api_server.as_deref().unwrap_or_default(),
                &config.node_name(),
                config.token.as_deref(),
            )
            .await;
            metrics.node_ready.store(ready, Ordering::Relaxed);
        }

        tokio::time::sleep(HEALTH_INTERVAL).await;
    }
}

fn build_ota(
    paths: &Paths,
    config: Option<NodeConfig>,
    metrics: Arc<Metrics>,
) -> Option<nrzube_agent::ota::Ota> {
    let config = config?;
    let registry_config = config.registry.clone()?;
    let channel = config.channel().to_string();

    let key_material = match &config.cosign_public_key {
        Some(key) => key.clone(),
        None => std::fs::read_to_string(&paths.cosign_pubkey).unwrap_or_default(),
    };

    let key = match TrustedKey::from_pem(&key_material) {
        Ok(key) => key,
        Err(err) => {
            log::error!(
                "no trusted cosign key: every update will be refused ({err}). \
                 Embed a public key in {}. Publishing the key in the node \
                 configuration is also possible.",
                paths.cosign_pubkey.display()
            );
            return None;
        }
    };

    let registry = match Registry::new(&registry_config) {
        Ok(registry) => registry,
        Err(err) => {
            log::error!("invalid registry configuration: {err:#}");
            return None;
        }
    };

    Some(nrzube_agent::ota::Ota::new(
        paths.clone(),
        registry,
        key,
        channel,
        metrics,
    ))
}

async fn ota_loop(ota: Option<nrzube_agent::ota::Ota>, metrics: Arc<Metrics>) {
    let Some(ota) = ota else {
        log::warn!("the OTA is disabled: no registry or no trusted key");
        return;
    };

    loop {
        // A pending slot means the node has just booted on a new rootfs: it
        // must be confirmed (or rolled back) before anything else.
        match ota.validate_pending().await {
            Ok(nrzube_agent::ota::Outcome::Confirmed) => {
                log::info!("the pending slot has been confirmed");
            }
            Ok(nrzube_agent::ota::Outcome::RolledBack) => {
                log::warn!("the pending slot has been rolled back");
            }
            Ok(_) => {}
            Err(err) => log::error!("cannot validate the pending slot: {err:#}"),
        }

        match ota.check_and_apply().await {
            Ok(nrzube_agent::ota::Outcome::Updated { from, to }) => {
                log::info!("updated from {from} to {to}");
            }
            Ok(_) => {}
            Err(err) => {
                // The registry may be unreachable: this must never break the
                // node (plan §7.4).
                metrics.update_failures.fetch_add(1, Ordering::Relaxed);
                log::error!("the OTA check failed (non fatal): {err:#}");
            }
        }

        tokio::time::sleep(OTA_INTERVAL).await;
    }
}
