//! The metrics exposed on 127.0.0.1:12249 (Prometheus text format).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The metrics of the agent.
#[derive(Debug, Default)]
pub struct Metrics {
    /// The agent is running.
    pub up: AtomicBool,
    /// The node configuration has been retrieved and persisted.
    pub configured: AtomicBool,
    /// containerd is healthy.
    pub containerd_healthy: AtomicBool,
    /// The kubelet is healthy.
    pub kubelet_healthy: AtomicBool,
    /// The Kubernetes node object is Ready.
    pub node_ready: AtomicBool,
    /// The slot the node is running from ("A" or "B").
    pub active_slot: AtomicU64,
    /// UNIX timestamp of the last successful update.
    pub last_update_timestamp: AtomicU64,
    /// Number of failed update attempts.
    pub update_failures: AtomicU64,
    /// Number of applied updates.
    pub updates_applied: AtomicU64,
    /// Number of rollbacks.
    pub rollbacks: AtomicU64,
}

/// Encode a slot name as a metric value.
const SLOT_A: u64 = 1;
const SLOT_B: u64 = 2;

impl Metrics {
    /// Create the metrics with their default values.
    pub fn new() -> Self {
        Self {
            up: AtomicBool::new(true),
            ..Default::default()
        }
    }

    /// Record the active slot.
    pub fn set_active_slot(&self, slot: &str) {
        let value = match slot {
            "A" => SLOT_A,
            "B" => SLOT_B,
            _ => 0,
        };
        self.active_slot.store(value, Ordering::Relaxed);
    }

    /// The Prometheus exposition of the metrics.
    pub fn render(&self) -> String {
        let slot = match self.active_slot.load(Ordering::Relaxed) {
            SLOT_A => "A",
            SLOT_B => "B",
            _ => "unknown",
        };
        let mut out = String::with_capacity(1024);

        out.push_str("# HELP nrzube_up Whether the agent is running.\n");
        out.push_str("# TYPE nrzube_up gauge\n");
        out.push_str(&format!("nrzube_up {}\n", self.up.load(Ordering::Relaxed) as u8));

        out.push_str("# HELP nrzube_node_configured Whether the node configuration has been persisted.\n");
        out.push_str("# TYPE nrzube_node_configured gauge\n");
        out.push_str(&format!(
            "nrzube_node_configured {}\n",
            self.configured.load(Ordering::Relaxed) as u8
        ));

        out.push_str("# HELP nrzube_containerd_healthy Whether containerd answers the gRPC health check.\n");
        out.push_str("# TYPE nrzube_containerd_healthy gauge\n");
        out.push_str(&format!(
            "nrzube_containerd_healthy {}\n",
            self.containerd_healthy.load(Ordering::Relaxed) as u8
        ));

        out.push_str("# HELP nrzube_kubelet_healthy Whether the kubelet answers on /healthz.\n");
        out.push_str("# TYPE nrzube_kubelet_healthy gauge\n");
        out.push_str(&format!(
            "nrzube_kubelet_healthy {}\n",
            self.kubelet_healthy.load(Ordering::Relaxed) as u8
        ));

        out.push_str("# HELP nrzube_node_ready Whether the Kubernetes node object is Ready.\n");
        out.push_str("# TYPE nrzube_node_ready gauge\n");
        out.push_str(&format!(
            "nrzube_node_ready {}\n",
            self.node_ready.load(Ordering::Relaxed) as u8
        ));

        out.push_str("# HELP nrzube_slot_active The slot the node is running from.\n");
        out.push_str("# TYPE nrzube_slot_active gauge\n");
        out.push_str(&format!("nrzube_slot_active{{slot=\"{slot}\"}} 1\n"));

        out.push_str("# HELP nrzube_update_last_success_timestamp_seconds Timestamp of the last applied update.\n");
        out.push_str("# TYPE nrzube_update_last_success_timestamp_seconds gauge\n");
        out.push_str(&format!(
            "nrzube_update_last_success_timestamp_seconds {}\n",
            self.last_update_timestamp.load(Ordering::Relaxed)
        ));

        out.push_str("# HELP nrzube_updates_applied_total Number of applied updates.\n");
        out.push_str("# TYPE nrzube_updates_applied_total counter\n");
        out.push_str(&format!(
            "nrzube_updates_applied_total {}\n",
            self.updates_applied.load(Ordering::Relaxed)
        ));

        out.push_str("# HELP nrzube_update_failures_total Number of failed update attempts.\n");
        out.push_str("# TYPE nrzube_update_failures_total counter\n");
        out.push_str(&format!(
            "nrzube_update_failures_total {}\n",
            self.update_failures.load(Ordering::Relaxed)
        ));

        out.push_str("# HELP nrzube_rollbacks_total Number of rollbacks to the previous slot.\n");
        out.push_str("# TYPE nrzube_rollbacks_total counter\n");
        out.push_str(&format!(
            "nrzube_rollbacks_total {}\n",
            self.rollbacks.load(Ordering::Relaxed)
        ));

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_the_expected_metrics() {
        let metrics = Metrics::new();
        metrics.set_active_slot("B");
        metrics.containerd_healthy.store(true, Ordering::Relaxed);

        let exposition = metrics.render();
        assert!(exposition.contains("nrzube_up 1"));
        assert!(exposition.contains("nrzube_slot_active{slot=\"B\"} 1"));
        assert!(exposition.contains("nrzube_containerd_healthy 1"));
        assert!(exposition.contains("nrzube_kubelet_healthy 0"));
    }
}
