// Phase 1 scaffold: config types and loader. Consumed starting Phase 2.
#![allow(dead_code)]

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub github: GithubConfig,
    pub signing: SigningConfig,
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub paths: PathsConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct GithubConfig {
    /// Personal access token. Scope: `actions:read`, `contents:write` on the
    /// source repo. Fine-grained tokens recommended.
    pub token: String,
    /// Source repo to poll, e.g. "nav-io/navio-core".
    pub source_repo: String,
    /// Workflow filename (e.g. "guix.yml") to filter runs.
    pub workflow: String,
    /// Refs to consider for signing. Defaults to master + v*.
    #[serde(default = "default_refs")]
    pub refs: Vec<String>,
}

fn default_refs() -> Vec<String> {
    vec!["refs/heads/master".into(), "refs/tags/v*".into()]
}

#[derive(Debug, Deserialize, Clone)]
pub struct SigningConfig {
    pub linux: LinuxSigningConfig,
    pub windows: WindowsSigningConfig,
    pub macos: MacosSigningConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LinuxSigningConfig {
    /// GPG key ID (long-form fingerprint preferred).
    pub gpg_key_id: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WindowsSigningConfig {
    /// Path to PKCS#12 cert bundle.
    pub pkcs12_path: PathBuf,
    /// Password for the PKCS#12 bundle. Plaintext on local disk (acceptable
    /// for a single-operator signer box; revisit when multi-operator).
    pub pkcs12_password: String,
    /// RFC3161 timestamp authority URL.
    #[serde(default = "default_tsa")]
    pub timestamp_url: String,
}

fn default_tsa() -> String {
    "http://timestamp.sectigo.com".into()
}

#[derive(Debug, Deserialize, Clone)]
pub struct MacosSigningConfig {
    /// codesign identity string, e.g. "Developer ID Application: Foo Bar (TEAMID)".
    pub identity: String,
    /// notarytool keychain profile name (set up via `xcrun notarytool store-credentials`).
    pub keychain_profile: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DaemonConfig {
    /// Poll interval in seconds.
    pub poll_interval_secs: u64,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            poll_interval_secs: 60,
        }
    }
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct PathsConfig {
    /// Override $XDG_DATA_HOME/navio-signer for workdir + db location.
    pub data_dir: Option<PathBuf>,
}

impl Config {
    pub fn load(explicit: Option<&Path>) -> Result<Self> {
        let path = match explicit {
            Some(p) => p.to_path_buf(),
            None => Self::default_path()?,
        };

        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading config at {}", path.display()))?;
        let cfg: Config = toml::from_str(&text)
            .with_context(|| format!("parsing config at {}", path.display()))?;
        Ok(cfg)
    }

    fn default_path() -> Result<PathBuf> {
        let cwd_config = std::env::current_dir()?.join("config.toml");
        if cwd_config.is_file() {
            return Ok(cwd_config);
        }

        let xdg = xdg::BaseDirectories::with_prefix("navio-signer");
        if let Some(p) = xdg.find_config_file("config.toml") {
            return Ok(p);
        }

        bail!(
            "no config.toml found in cwd or $XDG_CONFIG_HOME/navio-signer/. \
             pass --config explicitly or copy config.toml.example"
        );
    }
}
