//! Node configuration: retrieval, persistence and derived files.
//!
//! The configuration is delivered by the DHCP option 224 (a URL) and is
//! **persisted** in the state partition: from the second boot on, the node
//! never needs the network to be provisioned (plan §6.2).

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::util;

/// Filesystem layout of the running node.
#[derive(Debug, Clone)]
pub struct Paths {
    /// `/var/lib/nrzube`: persistent state of the agent.
    pub state_dir: PathBuf,
    /// The persisted node configuration.
    pub node_config: PathBuf,
    /// The slot the initramfs must boot.
    pub active_slot: PathBuf,
    /// The slot waiting for its health confirmation.
    pub pending_slot: PathBuf,
    /// Marker written by the agent once a slot is confirmed healthy.
    pub boot_ok: PathBuf,
    /// Where the downloaded updates are staged.
    pub staging_dir: PathBuf,
    /// The URL delivered by the DHCP option 224.
    pub dhcp_config_url: PathBuf,
    /// Kubelet state directory.
    pub kubelet_dir: PathBuf,
    /// CNI configuration directory.
    pub cni_dir: PathBuf,
    /// containerd registry configuration directory.
    pub containerd_certs_dir: PathBuf,
    /// The embedded cosign public key.
    pub cosign_pubkey: PathBuf,
    /// Runtime directory.
    pub run_dir: PathBuf,
    /// The persistent hostname file.
    pub hostname_file: PathBuf,
}

impl Default for Paths {
    fn default() -> Self {
        Self {
            state_dir: PathBuf::from("/var/lib/nrzube"),
            node_config: PathBuf::from("/var/lib/nrzube/node-config.json"),
            active_slot: PathBuf::from("/var/lib/nrzube/active-slot"),
            pending_slot: PathBuf::from("/var/lib/nrzube/pending-slot"),
            boot_ok: PathBuf::from("/var/lib/nrzube/boot-ok"),
            staging_dir: PathBuf::from("/var/lib/nrzube/staging"),
            dhcp_config_url: PathBuf::from("/run/nrzube/dhcp-config-url"),
            kubelet_dir: PathBuf::from("/var/lib/kubelet"),
            cni_dir: PathBuf::from("/etc/cni/net.d"),
            containerd_certs_dir: PathBuf::from("/etc/containerd/certs.d"),
            cosign_pubkey: PathBuf::from("/etc/nrzube/cosign.pub"),
            run_dir: PathBuf::from("/run/nrzube"),
            hostname_file: PathBuf::from("/etc/hostname"),
        }
    }
}

impl Paths {
    /// Build the paths of a node whose state lives in the given directory
    /// (used by the tests).
    pub fn under(root: &Path) -> Self {
        Self {
            state_dir: root.join("var/lib/nrzube"),
            node_config: root.join("var/lib/nrzube/node-config.json"),
            active_slot: root.join("var/lib/nrzube/active-slot"),
            pending_slot: root.join("var/lib/nrzube/pending-slot"),
            boot_ok: root.join("var/lib/nrzube/boot-ok"),
            staging_dir: root.join("var/lib/nrzube/staging"),
            dhcp_config_url: root.join("run/nrzube/dhcp-config-url"),
            kubelet_dir: root.join("var/lib/kubelet"),
            cni_dir: root.join("etc/cni/net.d"),
            containerd_certs_dir: root.join("etc/containerd/certs.d"),
            cosign_pubkey: root.join("etc/nrzube/cosign.pub"),
            run_dir: root.join("run/nrzube"),
            hostname_file: root.join("etc/hostname"),
        }
    }
}

/// The node configuration delivered by the DHCP option 224.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeConfig {
    /// URL of the Kubernetes API server.
    #[serde(default)]
    pub api_server: Option<String>,
    /// PEM certificate of the cluster CA.
    #[serde(default)]
    pub ca_cert: Option<String>,
    /// PEM client certificate (the node identity).
    #[serde(default)]
    pub client_cert: Option<String>,
    /// PEM client key.
    #[serde(default)]
    pub client_key: Option<String>,
    /// Bearer token, when the cluster uses tokens instead of certificates.
    #[serde(default)]
    pub token: Option<String>,
    /// Kubelet specific settings ("kubeletFlags" in the node configuration).
    #[serde(rename = "kubeletFlags", default)]
    pub kubelet_flags: KubeletConfig,
    /// CNI configuration, when the cluster does not provide one.
    #[serde(default)]
    pub cni: Option<CniConfig>,
    /// Registry holding the OTA artifacts.
    #[serde(default)]
    pub registry: Option<RegistryConfig>,
    /// cosign public key, when it differs from the embedded one.
    #[serde(default)]
    pub cosign_public_key: Option<String>,
    /// OTA channel.
    #[serde(default)]
    pub channel: Option<String>,
}

/// Kubelet settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KubeletConfig {
    /// Name of the node object.
    #[serde(default)]
    pub node_name: Option<String>,
    /// IP advertised by the node (stable, whatever the DHCP does).
    #[serde(default)]
    pub node_ip: Option<String>,
    /// DNS servers of the cluster.
    #[serde(default)]
    pub cluster_dns: Vec<String>,
    /// Pod CIDR of the node.
    #[serde(default)]
    pub pod_cidr: Option<String>,
    /// Prefix of the images pulled by the kubelet.
    #[serde(default)]
    pub registry_pull: Option<String>,
    /// Extra command line arguments for the kubelet.
    #[serde(default)]
    pub extra_args: Vec<String>,
}

/// CNI configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CniConfig {
    /// Name of the plugin (bridge, cilium...).
    pub name: String,
    /// Plugin specific configuration.
    #[serde(default)]
    pub config: serde_json::Value,
}

/// Registry holding the OTA artifacts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryConfig {
    /// Registry host, e.g. `ghcr.io/nrzube`.
    pub url: String,
    /// User name, when the registry requires credentials.
    #[serde(default)]
    pub username: Option<String>,
    /// Password or token, when the registry requires credentials.
    #[serde(default)]
    pub password: Option<String>,
    /// Pre-issued bearer token, when the registry provides one.
    #[serde(default)]
    pub token: Option<String>,
    /// Allow plain HTTP (development only).
    #[serde(default)]
    pub insecure: bool,
}

impl NodeConfig {
    /// Parse a configuration from JSON.
    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        let config: NodeConfig = serde_json::from_slice(bytes)
            .context("parsing the node configuration")?;
        config.validate()?;
        Ok(config)
    }

    /// Check that the configuration is usable.
    pub fn validate(&self) -> Result<()> {
        if self.api_server.is_none() {
            bail!("the node configuration has no apiServer");
        }
        if self.client_cert.is_none() && self.token.is_none() {
            bail!("the node configuration has neither a client certificate nor a token");
        }
        if self.client_cert.is_some() != self.client_key.is_some() {
            bail!("the node configuration has a client certificate without a key (or the other way round)");
        }
        Ok(())
    }

    /// Persist the configuration atomically.
    pub fn persist(&self, path: &Path) -> Result<()> {
        let serialized = serde_json::to_vec_pretty(self)?;
        util::write_atomic(path, &serialized)
    }

    /// Load a persisted configuration.
    pub fn load(path: &Path) -> Result<Option<Self>> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(Some(Self::from_slice(&bytes)?)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// The channel used to look for updates.
    pub fn channel(&self) -> &str {
        self.channel.as_deref().unwrap_or("stable")
    }

    /// The name of the node object.
    pub fn node_name(&self) -> String {
        self.kubelet_flags
            .node_name
            .clone()
            .unwrap_or_else(|| "nrzube".to_string())
    }

    /// Apply the node name to the system.
    ///
    /// The kubelet derives the name of the node object from the system
    /// hostname: both must agree with `kubeletFlags.nodeName`, otherwise the
    /// node registers under another name than the one the agent watches (and
    /// than the one the cluster expects).
    ///
    /// The name is also published in `/run/nrzube/node-name` so that the
    /// kubelet service can pass `--hostname-override` explicitly.
    pub fn apply_hostname(&self, paths: &Paths) -> Result<()> {
        let name = self.node_name();
        validate_dns1123(&name)
            .with_context(|| format!("the node name {name:?} is not a valid DNS-1123 label"))?;

        // The running hostname (UTS): what "uname -n", the logs and the
        // kubelet see. It needs the CAP_SYS_ADMIN capability (the agent
        // runs as root); a failure is not fatal because the kubelet is
        // started with an explicit --hostname-override.
        if let Err(err) = set_hostname(&name) {
            log::warn!("cannot set the running hostname: {err:#}");
        }

        // The persistent file: /etc is an overlay, the file lives in the
        // state partition and survives the reboots.
        util::write_atomic_str(&paths.hostname_file, &name)?;

        // The marker read by the kubelet service.
        util::write_atomic_str(&paths.run_dir.join("node-name"), &name)?;

        log::info!("the node name is {name}");
        Ok(())
    }

    /// Write the files derived from the configuration: kubelet PKI,
    /// kubeconfig, kubelet configuration, CNI configuration and the containerd
    /// registry credentials.
    pub fn write_derived_files(&self, paths: &Paths) -> Result<()> {
        self.write_kubelet_pki(paths)?;
        self.write_kubeconfig(paths)?;
        self.write_cni_config(paths)?;
        self.write_registry_credentials(paths)?;
        Ok(())
    }

    fn write_kubelet_pki(&self, paths: &Paths) -> Result<()> {
        let pki = paths.kubelet_dir.join("pki");
        std::fs::create_dir_all(&pki)
            .with_context(|| format!("creating {}", pki.display()))?;

        if let Some(ca) = &self.ca_cert {
            util::write_atomic(&pki.join("kubelet-client-ca.pem"), ca.as_bytes())?;
        }
        if let (Some(cert), Some(key)) = (&self.client_cert, &self.client_key) {
            // The kubelet expects the certificate and the key in the same file.
            let mut bundle = String::new();
            bundle.push_str(cert.trim());
            bundle.push('\n');
            bundle.push_str(key.trim());
            bundle.push('\n');
            util::write_atomic(&pki.join("kubelet-client-current.pem"), bundle.as_bytes())?;
        }
        Ok(())
    }

    fn write_kubeconfig(&self, paths: &Paths) -> Result<()> {
        let api_server = self.api_server.clone().unwrap_or_default();
        let mut kubeconfig = String::new();
        kubeconfig.push_str("apiVersion: v1\n");
        kubeconfig.push_str("kind: Config\n");
        kubeconfig.push_str("clusters:\n");
        kubeconfig.push_str("- name: nrzube\n");
        kubeconfig.push_str("  cluster:\n");
        kubeconfig.push_str(&format!("    server: {api_server}\n"));
        if let Some(ca) = &self.ca_cert {
            kubeconfig.push_str(&format!(
                "    certificate-authority-data: {}\n",
                BASE64.encode(ca.as_bytes())
            ));
        }
        kubeconfig.push_str("users:\n");
        kubeconfig.push_str("- name: nrzube\n");
        kubeconfig.push_str("  user:\n");
        if let (Some(cert), Some(key)) = (&self.client_cert, &self.client_key) {
            kubeconfig.push_str(&format!(
                "    client-certificate-data: {}\n",
                BASE64.encode(cert.as_bytes())
            ));
            kubeconfig.push_str(&format!(
                "    client-key-data: {}\n",
                BASE64.encode(key.as_bytes())
            ));
        } else if let Some(token) = &self.token {
            kubeconfig.push_str(&format!("    token: {token}\n"));
        }
        kubeconfig.push_str("contexts:\n");
        kubeconfig.push_str("- name: nrzube\n");
        kubeconfig.push_str("  context:\n");
        kubeconfig.push_str("    cluster: nrzube\n");
        kubeconfig.push_str("    user: nrzube\n");
        kubeconfig.push_str("current-context: nrzube\n");

        util::write_atomic(
            &paths.kubelet_dir.join("kubeconfig"),
            kubeconfig.as_bytes(),
        )?;
        util::write_atomic(
            &paths.kubelet_dir.join("config.yaml"),
            self.kubelet_configuration().as_bytes(),
        )?;
        Ok(())
    }

    /// The KubeletConfiguration of the node.
    pub fn kubelet_configuration(&self) -> String {
        let mut config = String::new();
        config.push_str("apiVersion: kubelet.config.k8s.io/v1beta1\n");
        config.push_str("kind: KubeletConfiguration\n");
        config.push_str("authentication:\n");
        config.push_str("  anonymous:\n");
        config.push_str("    enabled: false\n");
        config.push_str("  webhook:\n");
        config.push_str("    enabled: true\n");
        config.push_str("    cacheTTL: 2m0s\n");
        config.push_str("  x509:\n");
        config.push_str("    clientCAFile: /var/lib/kubelet/pki/kubelet-client-ca.pem\n");
        config.push_str("authorization:\n");
        config.push_str("  mode: Webhook\n");
        // No systemd on the node: the cgroup manager is the cgroupfs one.
        config.push_str("cgroupDriver: cgroupfs\n");
        config.push_str("clusterDomain: cluster.local\n");
        if !self.kubelet_flags.cluster_dns.is_empty() {
            config.push_str("clusterDNS:\n");
            for dns in &self.kubelet_flags.cluster_dns {
                config.push_str(&format!("- {dns}\n"));
            }
        }
        if let Some(pod_cidr) = &self.kubelet_flags.pod_cidr {
            config.push_str(&format!("podCIDR: {pod_cidr}\n"));
        }
        config.push_str("nodeStatusUpdateFrequency: 10s\n");
        // dhcpcd writes /etc/resolv.conf (there is no systemd-resolved here).
        config.push_str("resolvConf: /etc/resolv.conf\n");
        config.push_str("rotateCertificates: true\n");
        config.push_str("runtimeRequestTimeout: 2m0s\n");
        config
    }

    fn write_cni_config(&self, paths: &Paths) -> Result<()> {
        let Some(cni) = &self.cni else {
            return Ok(());
        };
        std::fs::create_dir_all(&paths.cni_dir)
            .with_context(|| format!("creating {}", paths.cni_dir.display()))?;

        let mut list = serde_json::json!({
            "name": cni.name,
            "cniVersion": "1.0.0",
            "plugins": [ cni.config.clone() ],
        });
        if let Some(pod_cidr) = &self.kubelet_flags.pod_cidr {
            list["plugins"][0]["subnet"] = serde_json::Value::String(pod_cidr.clone());
        }
        let serialized = serde_json::to_vec_pretty(&list)?;
        util::write_atomic(&paths.cni_dir.join("10-nrzube.conflist"), &serialized)
    }

    fn write_registry_credentials(&self, paths: &Paths) -> Result<()> {
        let Some(registry) = &self.registry else {
            return Ok(());
        };
        let (Some(username), Some(password)) = (&registry.username, &registry.password) else {
            return Ok(());
        };

        let dir = paths.containerd_certs_dir.join(&registry.url);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("creating {}", dir.display()))?;

        let hosts = format!(
            "server = \"https://{registry}\"\n\n\
             [host.\"https://{registry}\"]\n  \
             capabilities = [\"pull\", \"resolve\"]\n  \
             username = \"{username}\"\n  \
             password = \"{password}\"\n",
            registry = registry.url
        );
        util::write_atomic(&dir.join("hosts.toml"), hosts.as_bytes())
    }
}

/// The URL of the node configuration, as delivered by the DHCP option 224.
pub fn dhcp_config_url(paths: &Paths) -> Option<String> {
    let url = util::read_trimmed(&paths.dhcp_config_url)?;
    if url.starts_with("http://") || url.starts_with("https://") {
        Some(url)
    } else {
        None
    }
}

/// Check that a name is a valid DNS-1123 label (the Kubernetes node names).
fn validate_dns1123(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 63 {
        bail!("a DNS-1123 label must be between 1 and 63 characters long");
    }
    let valid = name
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if !valid {
        bail!("a DNS-1123 label only holds lowercase letters, digits and dashes");
    }
    if name.starts_with('-') || name.ends_with('-') {
        bail!("a DNS-1123 label cannot start or end with a dash");
    }
    Ok(())
}

/// Set the running hostname (UTS namespace).
fn set_hostname(name: &str) -> Result<()> {
    let bytes = name.as_bytes();
    // SAFETY: the pointer and the length describe a valid byte string, which
    // sethostname copies before returning.
    let result = unsafe {
        libc::sethostname(bytes.as_ptr() as *const libc::c_char, bytes.len())
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error()).context("sethostname");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> NodeConfig {
        serde_json::from_str(
            r#"{
                "apiServer": "https://10.0.0.1:6443",
                "caCert": "ca",
                "clientCert": "cert",
                "clientKey": "key",
                "kubeletFlags": { "nodeName": "nrzube-01", "nodeIp": "10.0.0.50",
                                  "clusterDns": ["10.96.0.10"], "podCidr": "10.244.0.0/16" },
                "registry": { "url": "ghcr.io/nrzube", "username": "u", "password": "p" }
            }"#,
        )
        .expect("valid configuration")
    }

    #[test]
    fn parses_the_documented_schema() {
        let config = sample();
        assert_eq!(config.api_server.as_deref(), Some("https://10.0.0.1:6443"));
        assert_eq!(config.node_name(), "nrzube-01");
        assert_eq!(config.kubelet_flags.node_ip.as_deref(), Some("10.0.0.50"));
        assert_eq!(config.kubelet_flags.cluster_dns, vec!["10.96.0.10".to_string()]);
        assert_eq!(config.channel(), "stable");
    }

    #[test]
    fn rejects_an_incomplete_configuration() {
        let config: NodeConfig = serde_json::from_str(r#"{"apiServer":"https://x"}"#).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn persists_and_loads() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let config = sample();
        config.persist(&paths.node_config).unwrap();
        let loaded = NodeConfig::load(&paths.node_config).unwrap().unwrap();
        assert_eq!(loaded.api_server, config.api_server);
    }

    #[test]
    fn writes_the_derived_files() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        sample().write_derived_files(&paths).unwrap();

        assert!(paths.kubelet_dir.join("kubeconfig").exists());
        assert!(paths.kubelet_dir.join("config.yaml").exists());
        assert!(paths.kubelet_dir.join("pki/kubelet-client-current.pem").exists());

        let kubeconfig = std::fs::read_to_string(paths.kubelet_dir.join("kubeconfig")).unwrap();
        assert!(kubeconfig.contains("server: https://10.0.0.1:6443"));
        assert!(kubeconfig.contains("client-certificate-data:"));

        let kubelet = std::fs::read_to_string(paths.kubelet_dir.join("config.yaml")).unwrap();
        assert!(kubelet.contains("cgroupDriver: cgroupfs"));
        assert!(kubelet.contains("- 10.96.0.10"));
    }

    #[test]
    fn applies_the_node_name() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let config = sample();

        config.apply_hostname(&paths).unwrap();

        assert_eq!(
            util::read_trimmed(&paths.hostname_file).as_deref(),
            Some("nrzube-01")
        );
        assert_eq!(
            util::read_trimmed(&paths.run_dir.join("node-name")).as_deref(),
            Some("nrzube-01")
        );
    }

    #[test]
    fn rejects_an_invalid_node_name() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());

        for invalid in ["", "Nrzube-01", "nrzube_01", "-nrzube", "nrzube-", &"a".repeat(64)] {
            let mut config = sample();
            config.kubelet_flags.node_name = Some(invalid.to_string());
            assert!(
                config.apply_hostname(&paths).is_err(),
                "{invalid:?} should have been rejected"
            );
        }
    }
}
