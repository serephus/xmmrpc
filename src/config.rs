// SPDX-License-Identifier: GPL-2.0 OR BSD-3-Clause
//! Command-line and TOML configuration.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use palc::Parser;
use serde::Deserialize;

use crate::error::{Error, Result};

/// Default RPC control port exposed by the `iosm` driver.
pub const DEFAULT_RPC_PORT: &str = "/dev/wwan0xmmrpc0";
/// Default WWAN network interface created by `iosm`.
pub const DEFAULT_INTERFACE: &str = "wwan0";
/// Default metric for the installed default route.
pub const DEFAULT_METRIC: u32 = 1000;
/// Default interval between address queries, in seconds.
pub const DEFAULT_FETCH_INTERVAL_SECS: u64 = 1;
/// Default time to wait for an address, in seconds.
pub const DEFAULT_WAIT_SECS: u64 = 120;
/// Default firmware data-channel path.
pub const DEFAULT_DATACHANNEL: &str = "/sioscc/PCIE/IOSM/IPS/0";
/// Default resolver configuration file.
pub const DEFAULT_RESOLV_CONF: &str = "/etc/resolv.conf";

/// Command-line interface.
///
/// Bring up an Intel XMM7360 (Fibocom L850-GL) data connection through the
/// in-tree `iosm` driver.
#[derive(Debug, Parser)]
#[command(name = "xmmrpc")]
pub struct Cli {
    /// Path to a TOML configuration file.
    #[arg(short, long)]
    pub config: Option<PathBuf>,

    /// Network provider APN.
    #[arg(short, long)]
    pub apn: Option<String>,

    /// WWAN network interface created by iosm.
    #[arg(short, long)]
    pub interface: Option<String>,

    /// XMM RPC control port.
    #[arg(long)]
    pub rpc_port: Option<PathBuf>,

    /// Install the modem as the default route (`true`/`false`).
    #[arg(long)]
    pub default_route: Option<bool>,

    /// Metric for the default route (higher is lower priority).
    #[arg(short, long)]
    pub metric: Option<u32>,

    /// Seconds between attempts to fetch the assigned IP address.
    #[arg(short = 't', long)]
    pub ip_fetch_interval: Option<u64>,

    /// Seconds to wait for the network to assign an address.
    #[arg(long)]
    pub ip_wait: Option<u64>,

    /// Append modem-provided DNS servers to the resolver configuration
    /// (`true`/`false`).
    #[arg(long)]
    pub write_resolv: Option<bool>,

    /// Resolver configuration file to append DNS servers to.
    #[arg(long)]
    pub resolv_conf: Option<PathBuf>,

    /// Firmware data-channel path.
    #[arg(long)]
    pub datachannel_path: Option<String>,

    /// Print the version and exit.
    #[arg(long)]
    pub version: bool,
}

/// Values read from a TOML configuration file.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FileConfig {
    /// Network provider APN.
    pub apn: Option<String>,
    /// WWAN network interface.
    pub interface: Option<String>,
    /// RPC control port.
    pub rpc_port: Option<PathBuf>,
    /// Whether to install the modem as the default route.
    pub default_route: Option<bool>,
    /// Default route metric.
    pub metric: Option<u32>,
    /// Seconds between address queries.
    pub ip_fetch_interval: Option<u64>,
    /// Seconds to wait for an address.
    pub ip_wait: Option<u64>,
    /// Whether to append DNS servers to the resolver configuration.
    pub write_resolv: Option<bool>,
    /// Resolver configuration file.
    pub resolv_conf: Option<PathBuf>,
    /// Firmware data-channel path.
    pub datachannel_path: Option<String>,
}

/// Fully resolved runtime settings.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Network provider APN.
    pub apn: String,
    /// WWAN network interface.
    pub interface: String,
    /// RPC control port.
    pub rpc_port: PathBuf,
    /// Whether to install the modem as the default route.
    pub default_route: bool,
    /// Default route metric.
    pub metric: u32,
    /// Interval between address queries.
    pub ip_fetch_interval: Duration,
    /// Time to wait for an address.
    pub ip_wait: Duration,
    /// Whether to append DNS servers to the resolver configuration.
    pub write_resolv: bool,
    /// Resolver configuration file.
    pub resolv_conf: PathBuf,
    /// Firmware data-channel path.
    pub datachannel_path: String,
}

impl Settings {
    /// Merge defaults, the configuration file and command-line arguments.
    pub fn resolve(cli: &Cli) -> Result<Self> {
        let file = load_file(cli.config.as_deref())?;

        let apn = cli
            .apn
            .clone()
            .or(file.apn)
            .filter(|apn| !apn.is_empty())
            .ok_or_else(|| {
                Error::Config(
                    "no APN configured (pass --apn or set `apn = \"...\"` in the config file)"
                        .into(),
                )
            })?;

        Ok(Self {
            apn,
            interface: cli
                .interface
                .clone()
                .or(file.interface)
                .unwrap_or_else(|| DEFAULT_INTERFACE.to_string()),
            rpc_port: cli
                .rpc_port
                .clone()
                .or(file.rpc_port)
                .unwrap_or_else(|| PathBuf::from(DEFAULT_RPC_PORT)),
            default_route: cli.default_route.or(file.default_route).unwrap_or(true),
            metric: cli.metric.or(file.metric).unwrap_or(DEFAULT_METRIC),
            ip_fetch_interval: Duration::from_secs(
                cli.ip_fetch_interval
                    .or(file.ip_fetch_interval)
                    .unwrap_or(DEFAULT_FETCH_INTERVAL_SECS),
            ),
            ip_wait: Duration::from_secs(cli.ip_wait.or(file.ip_wait).unwrap_or(DEFAULT_WAIT_SECS)),
            write_resolv: cli.write_resolv.or(file.write_resolv).unwrap_or(true),
            resolv_conf: cli
                .resolv_conf
                .clone()
                .or(file.resolv_conf)
                .unwrap_or_else(|| PathBuf::from(DEFAULT_RESOLV_CONF)),
            datachannel_path: cli
                .datachannel_path
                .clone()
                .or(file.datachannel_path)
                .unwrap_or_else(|| DEFAULT_DATACHANNEL.to_string()),
        })
    }
}

/// Candidate configuration files searched when `--config` is not given.
pub fn default_config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        paths.push(cwd.join("xmmrpc.toml"));
    }
    paths.push(PathBuf::from("/etc/xmmrpc.toml"));
    paths
}

fn load_file(explicit: Option<&Path>) -> Result<FileConfig> {
    if let Some(path) = explicit {
        return read_config(path);
    }

    for candidate in default_config_paths() {
        if candidate.exists() {
            return read_config(&candidate);
        }
    }
    Ok(FileConfig::default())
}

fn read_config(path: &Path) -> Result<FileConfig> {
    let text = fs::read_to_string(path)
        .map_err(|source| Error::Config(format!("cannot read {}: {source}", path.display())))?;
    toml::from_str(&text)
        .map_err(|source| Error::Config(format!("invalid TOML in {}: {source}", path.display())))
}
