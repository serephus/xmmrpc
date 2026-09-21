// SPDX-License-Identifier: GPL-2.0 OR BSD-3-Clause
//! `xmmrpc` command-line entry point.

use std::process::ExitCode;

use palc::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

use xmmrpc::Error;
use xmmrpc::config::{Cli, Settings};
use xmmrpc::modem;
use xmmrpc::net;
use xmmrpc::transport::Rpc;

fn main() -> ExitCode {
    init_tracing();
    let cli = Cli::parse();

    if cli.version {
        println!("xmmrpc {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    let settings = match Settings::resolve(&cli) {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(error.exit_code());
        }
    };

    match run(&settings) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(error.exit_code())
        }
    }
}

fn run(settings: &Settings) -> Result<(), Error> {
    let mut rpc = Rpc::open(&settings.rpc_port)?;
    info!(port = %settings.rpc_port.display(), "opened RPC control port");

    modem::wait_ready(&mut rpc)?;
    modem::initialize(&mut rpc)?;

    modem::fcc_unlock(&mut rpc)?;
    modem::set_mode(&mut rpc, 1)?;

    match modem::firmware_version(&mut rpc) {
        Ok(version) => info!(version, "firmware"),
        Err(error) => tracing::warn!(%error, "could not read the firmware version"),
    }

    if !modem::attach(&mut rpc, &settings.apn)? {
        return Err(Error::AttachRefused);
    }
    info!(apn = %settings.apn, "attached to the packet data network");

    let Some((address, dns)) =
        modem::wait_for_ip(&mut rpc, settings.ip_wait, settings.ip_fetch_interval)?
    else {
        return Err(Error::NoIp);
    };
    info!(%address, "assigned IP address");
    info!(v4 = ?dns.v4, v6 = ?dns.v6, "DNS servers");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(net::configure(
        &settings.interface,
        address,
        settings.metric,
        settings.default_route,
    ))?;
    info!(interface = %settings.interface, "configured network interface");

    if settings.write_resolv {
        net::append_resolv_conf(&dns, &settings.resolv_conf)?;
    }

    modem::open_data_channel(&mut rpc, &settings.datachannel_path)?;
    info!("data channel is up");
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}
