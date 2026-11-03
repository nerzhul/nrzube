//! A minimal OCI distribution client.
//!
//! Only what the OTA needs: pull a manifest by tag and download a blob by
//! digest, with the Docker Registry v2 authentication flow (anonymous, basic
//! credentials or pre-issued bearer token).

use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

use crate::config::RegistryConfig;

/// An OCI descriptor.
#[derive(Debug, Clone, Deserialize)]
pub struct Descriptor {
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub digest: String,
    #[serde(default)]
    pub size: u64,
}

/// An OCI image manifest.
#[derive(Debug, Clone, Deserialize)]
pub struct OciManifest {
    #[serde(rename = "schemaVersion", default)]
    pub schema_version: u32,
    #[serde(rename = "mediaType", default)]
    pub media_type: Option<String>,
    #[serde(default)]
    pub config: Option<Descriptor>,
    #[serde(default)]
    pub layers: Vec<Descriptor>,
}

impl OciManifest {
    /// The layer with the given media type.
    pub fn layer(&self, media_type: &str) -> Option<&Descriptor> {
        self.layers
            .iter()
            .find(|layer| layer.media_type == media_type)
    }
}

/// The credentials used against the registry.
#[derive(Debug, Clone)]
pub enum Auth {
    /// Anonymous access (public registry).
    Anonymous,
    /// Basic authentication.
    Basic {
        username: String,
        password: String,
    },
    /// A pre-issued bearer token.
    Bearer(String),
}

/// A client of an OCI registry.
pub struct Registry {
    client: reqwest::Client,
    /// Scheme of the registry (https, or http for a development registry).
    scheme: String,
    /// Registry host, e.g. `ghcr.io`.
    host: String,
    /// Repository, e.g. `nrzube/nrzube`.
    repository: String,
    auth: Auth,
    token: tokio::sync::RwLock<Option<String>>,
}

impl Registry {
    /// Build a client from the node configuration.
    pub fn new(config: &RegistryConfig) -> Result<Self> {
        let mut url = config.url.trim().to_string();
        while url.ends_with('/') {
            url.pop();
        }
        let (scheme, rest) = match url.split_once("://") {
            Some((scheme, rest)) => (scheme.to_string(), rest.to_string()),
            None => ("https".to_string(), url.clone()),
        };
        let scheme = if config.insecure && scheme == "https" {
            log::warn!("the registry {url} is used in plain HTTP (insecure mode)");
            "http".to_string()
        } else {
            scheme
        };
        let (host, repository) = match rest.split_once('/') {
            Some((host, repository)) => (host.to_string(), repository.to_string()),
            None => (rest.clone(), String::new()),
        };

        if repository.is_empty() {
            bail!("the registry configuration has no repository");
        }

        let auth = if let Some(token) = &config.token {
            Auth::Bearer(token.clone())
        } else if let (Some(username), Some(password)) = (&config.username, &config.password) {
            Auth::Basic {
                username: username.clone(),
                password: password.clone(),
            }
        } else {
            Auth::Anonymous
        };

        let client = reqwest::Client::builder()
            .user_agent(concat!("nrzube-agent/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .context("building the HTTP client")?;

        Ok(Self {
            client,
            scheme,
            host,
            repository,
            auth,
            token: tokio::sync::RwLock::new(None),
        })
    }

    fn scheme(&self) -> &str {
        &self.scheme
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}://{}/v2/{}/{}",
            self.scheme(),
            self.host,
            self.repository,
            path
        )
    }

    /// Fetch the manifest of a tag.
    pub async fn manifest(&self, tag: &str) -> Result<OciManifest> {
        let url = self.url(&format!("manifests/{tag}"));
        let accept = concat!(
            "application/vnd.oci.image.manifest.v1+json, ",
            "application/vnd.docker.distribution.manifest.v2+json, ",
            "application/vnd.oci.image.index.v1+json"
        );

        let bytes = self.get_bytes(&url, accept).await?;
        serde_json::from_slice(&bytes).context("decoding the OCI manifest")
    }

    /// Download a blob into a file, returning the number of bytes written.
    pub async fn download_blob(&self, digest: &str, destination: &Path) -> Result<u64> {
        let url = self.url(&format!("blobs/{digest}"));
        let request = self.authorized_request(&url).await?;
        let response = request
            .send()
            .await
            .with_context(|| format!("downloading {url}"))?
            .error_for_status()
            .with_context(|| format!("downloading {url}"))?;

        let mut file = tokio::fs::File::create(destination)
            .await
            .with_context(|| format!("creating {}", destination.display()))?;
        let mut stream = response.bytes_stream();
        let mut written: u64 = 0;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("reading the blob")?;
            written += chunk.len() as u64;
            tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
                .await
                .context("writing the blob")?;
        }
        tokio::io::AsyncWriteExt::flush(&mut file)
            .await
            .context("flushing the blob")?;
        file.sync_all().await.context("syncing the blob")?;
        Ok(written)
    }

    /// Download a blob in memory.
    pub async fn blob(&self, digest: &str) -> Result<Vec<u8>> {
        let url = self.url(&format!("blobs/{digest}"));
        let request = self.authorized_request(&url).await?;
        let response = request
            .send()
            .await
            .with_context(|| format!("downloading {url}"))?
            .error_for_status()
            .with_context(|| format!("downloading {url}"))?;
        Ok(response.bytes().await.context("reading the blob")?.to_vec())
    }

    async fn get_bytes(&self, url: &str, accept: &str) -> Result<Vec<u8>> {
        let request = self
            .authorized_request(url)
            .await?
            .header("accept", accept);
        let response = request
            .send()
            .await
            .with_context(|| format!("GET {url}"))?
            .error_for_status()
            .with_context(|| format!("GET {url}"))?;
        Ok(response.bytes().await.context("reading the response")?.to_vec())
    }

    /// Build a request carrying the right authorization header.
    async fn authorized_request(&self, url: &str) -> Result<reqwest::RequestBuilder> {
        let mut request = self.client.get(url);

        if let Some(token) = self.token.read().await.clone() {
            request = request.bearer_auth(token);
            return Ok(request);
        }

        match &self.auth {
            Auth::Anonymous => Ok(request),
            Auth::Bearer(token) => {
                request = request.bearer_auth(token);
                Ok(request)
            }
            Auth::Basic { username, password } => {
                request = request.basic_auth(username, Some(password));
                Ok(request)
            }
        }
    }

    /// Fetch (and cache) a registry token.
    ///
    /// The Docker registry v2 flow: the registry answers 401 with the
    /// authentication realm, the token is then requested from that realm.
    pub async fn authenticate(&self, scope: &str) -> Result<String> {
        if let Some(token) = self.token.read().await.clone() {
            return Ok(token);
        }

        let url = format!("{}://{}/v2/", self.scheme(), self.host);
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;

        let challenge = response
            .headers()
            .get("www-authenticate")
            .and_then(|value| value.to_str().ok())
            .map(|value| value.to_string())
            .unwrap_or_default();

        let parameters = parse_challenge(&challenge);
        let realm = parameters
            .get("realm")
            .cloned()
            .context("the registry did not return an authentication realm")?;

        let mut token_url = format!("{realm}?");
        if let Some(service) = parameters.get("service") {
            token_url.push_str(&format!("service={service}&"));
        }
        token_url.push_str(&format!("scope={scope}"));

        let mut request = self.client.get(&token_url);
        if let Auth::Basic { username, password } = &self.auth {
            request = request.basic_auth(username, Some(password));
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("GET {token_url}"))?
            .error_for_status()
            .with_context(|| format!("GET {token_url}"))?;

        let body: TokenResponse = response
            .json()
            .await
            .context("decoding the token response")?;
        let token = body
            .token
            .or(body.access_token)
            .context("the registry returned no token")?;

        *self.token.write().await = Some(token.clone());
        Ok(token)
    }

    /// The scope needed to pull the artifacts of this repository.
    pub fn pull_scope(&self) -> String {
        format!("repository:{}:pull", self.repository)
    }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    #[serde(rename = "access_token")]
    access_token: Option<String>,
}

/// Parse a `WWW-Authenticate: Bearer realm="...",service="..."` header.
fn parse_challenge(challenge: &str) -> HashMap<String, String> {
    let mut parameters = HashMap::new();
    let Some(parameters_part) = challenge.split_once(' ').map(|(_, rest)| rest) else {
        return parameters;
    };

    let mut rest = parameters_part;
    while let Some((key, remainder)) = rest.split_once('=') {
        let key = key.trim().to_string();
        let remainder = remainder.trim_start();
        if let Some(value) = remainder.strip_prefix('"') {
            match value.find('"') {
                Some(end) => {
                    parameters.insert(key, value[..end].to_string());
                    rest = value[end + 1..].trim_start().trim_start_matches(',').trim_start();
                }
                None => break,
            }
        } else {
            let end = remainder.find(',').unwrap_or(remainder.len());
            parameters.insert(key, remainder[..end].trim().to_string());
            rest = remainder[end..].trim_start().trim_start_matches(',').trim_start();
        }
    }
    parameters
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_bearer_challenge() {
        let parameters = parse_challenge(
            "Bearer realm=\"https://ghcr.io/token\",service=\"ghcr.io\",scope=\"repository:nrzube/nrzube:pull\"",
        );
        assert_eq!(
            parameters.get("realm").map(String::as_str),
            Some("https://ghcr.io/token")
        );
        assert_eq!(parameters.get("service").map(String::as_str), Some("ghcr.io"));
        assert_eq!(
            parameters.get("scope").map(String::as_str),
            Some("repository:nrzube/nrzube:pull")
        );
    }

    #[test]
    fn parses_an_empty_challenge() {
        assert!(parse_challenge("").is_empty());
        assert!(parse_challenge("Bearer").is_empty());
    }

    #[test]
    fn splits_the_registry_url() {
        let config = RegistryConfig {
            url: "ghcr.io/nrzube".to_string(),
            username: None,
            password: None,
            token: None,
            insecure: false,
        };
        let registry = Registry::new(&config).expect("valid registry");
        assert_eq!(registry.host, "ghcr.io");
        assert_eq!(registry.repository, "nrzube");
        assert_eq!(
            registry.url("manifests/nrzube-stable"),
            "https://ghcr.io/v2/nrzube/manifests/nrzube-stable"
        );
    }

    #[test]
    fn rejects_a_registry_without_repository() {
        let config = RegistryConfig {
            url: "ghcr.io".to_string(),
            username: None,
            password: None,
            token: None,
            insecure: false,
        };
        assert!(Registry::new(&config).is_err());
    }
}
