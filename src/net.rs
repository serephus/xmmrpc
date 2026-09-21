// SPDX-License-Identifier: GPL-2.0 OR BSD-3-Clause
//! Network interface, route and resolver configuration.

use std::fs::OpenOptions;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;

use futures_util::TryStreamExt;
use rtnetlink::{Handle, LinkUnspec, RouteMessageBuilder, new_connection};

use crate::error::{Error, Result};
use crate::requests::DnsServers;

/// Flush existing addresses, bring the link up, assign `address` and, unless
/// disabled, install a default route with the given metric.
pub async fn configure(
    interface: &str,
    address: Ipv4Addr,
    metric: u32,
    default_route: bool,
) -> Result<()> {
    let (connection, handle, _) = new_connection().map_err(Error::netlink)?;
    tokio::spawn(connection);

    let index = link_index(&handle, interface).await?;
    flush_addresses(&handle, index).await?;

    handle
        .link()
        .set(LinkUnspec::new_with_index(index).up().build())
        .execute()
        .await
        .map_err(Error::netlink)?;

    handle
        .address()
        .add(index, IpAddr::V4(address), 32)
        .execute()
        .await
        .map_err(Error::netlink)?;

    if default_route {
        let route = RouteMessageBuilder::<Ipv4Addr>::new()
            .destination_prefix(Ipv4Addr::UNSPECIFIED, 0)
            .output_interface(index)
            .priority(metric)
            .build();
        handle
            .route()
            .add(route)
            .execute()
            .await
            .map_err(Error::netlink)?;
    }

    Ok(())
}

async fn link_index(handle: &Handle, name: &str) -> Result<u32> {
    let mut links = handle.link().get().match_name(name.to_string()).execute();
    match links.try_next().await.map_err(Error::netlink)? {
        Some(link) => Ok(link.header.index),
        None => Err(Error::InterfaceNotFound(name.to_string())),
    }
}

async fn flush_addresses(handle: &Handle, index: u32) -> Result<()> {
    let mut addresses = handle
        .address()
        .get()
        .set_link_index_filter(index)
        .execute();
    while let Some(address) = addresses.try_next().await.map_err(Error::netlink)? {
        handle
            .address()
            .del(address)
            .execute()
            .await
            .map_err(Error::netlink)?;
    }
    Ok(())
}

/// Append modem-provided DNS servers to the resolver configuration file.
pub fn append_resolv_conf(servers: &DnsServers, path: &Path) -> Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "\n# Added by xmmrpc")?;
    for server in &servers.v4 {
        writeln!(file, "nameserver {server}")?;
    }
    for server in &servers.v6 {
        writeln!(file, "nameserver {server}")?;
    }
    Ok(())
}
