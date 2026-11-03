//! nrzube-agent: supervision, bootstrap and signed OTA agent of the nrzube
//! Kubernetes worker.
//!
//! The agent is a single binary started by s6. It:
//!   - bootstraps the node from the URL delivered by the DHCP option 224 and
//!     persists the configuration in the state partition, so that the node
//!     boots and rejoins the cluster without any network access;
//!   - watches containerd, kubelet and the Kubernetes node object;
//!   - exposes `/healthz` and `/metrics` on 127.0.0.1:12249;
//!   - applies the OTA updates published in an OCI registry, after checking
//!     the cosign signature and the SHA-256 of every component, with an
//!     automatic A/B rollback.

pub mod config;
pub mod cosign;
pub mod health;
pub mod logging;
pub mod metrics;
pub mod ota;
pub mod registry;
pub mod server;
pub mod slots;
pub mod util;

pub use config::{NodeConfig, Paths};
