// SPDX-License-Identifier: GPL-2.0 OR BSD-3-Clause
//! Request/response payload builders for the XMM7360 RPC protocol.
//!
//! The wire encodings are ported byte-for-byte from the reverse-engineered
//! `xmm7360-pci` implementation.

use std::net::{Ipv4Addr, Ipv6Addr};

use crate::codec::{self, Arg, CodecError, Value};

/// `UtaMsNetAttachReq`.
pub fn net_attach() -> Vec<u8> {
    codec::pack(
        "BLLLLHHLL",
        &[
            Arg::Int(0),
            Arg::Int(0),
            Arg::Int(0),
            Arg::Int(0),
            Arg::Int(0),
            Arg::Int(0xffff),
            Arg::Int(0xffff),
            Arg::Int(0),
            Arg::Int(0),
        ],
    )
    .expect("static format")
}

/// `UtaMsCallPsGetNegIpAddrReq`.
pub fn get_neg_ip() -> Vec<u8> {
    codec::pack("BLL", &[Arg::Int(0), Arg::Int(0), Arg::Int(0)]).expect("static format")
}

/// `UtaMsCallPsGetNegotiatedDnsReq`.
pub fn get_neg_dns() -> Vec<u8> {
    codec::pack("BLL", &[Arg::Int(0), Arg::Int(0), Arg::Int(0)]).expect("static format")
}

/// `UtaMsCallPsConnectReq`.
pub fn ps_connect() -> Vec<u8> {
    codec::pack(
        "BLLL",
        &[Arg::Int(0), Arg::Int(6), Arg::Int(0), Arg::Int(0)],
    )
    .expect("static format")
}

/// `UtaRPCPsConnectToDatachannelReq`.
pub fn connect_to_datachannel(path: &str) -> Result<Vec<u8>, CodecError> {
    let mut buffer = Vec::with_capacity(path.len() + 1);
    buffer.extend_from_slice(path.as_bytes());
    buffer.push(0);
    codec::pack("s24", &[Arg::Bytes(&buffer)])
}

/// `UtaSysGetInfo`.
pub fn sys_get_info(index: u32) -> Vec<u8> {
    codec::pack("Ls0L", &[Arg::Int(0), Arg::Bytes(&[]), Arg::Int(index)]).expect("static format")
}

/// Decode `UtaMsCallPsGetNegIpAddrRspCb` into its three candidate addresses.
pub fn parse_neg_ip(body: &[u8]) -> Result<[Ipv4Addr; 3], CodecError> {
    let values = codec::unpack("nsnnnn", body)?;
    let addresses = values
        .get(1)
        .and_then(Value::as_bytes)
        .ok_or(CodecError::MissingArgument)?;
    if addresses.len() < 12 {
        return Err(CodecError::UnexpectedEof);
    }
    Ok([
        Ipv4Addr::new(addresses[0], addresses[1], addresses[2], addresses[3]),
        Ipv4Addr::new(addresses[4], addresses[5], addresses[6], addresses[7]),
        Ipv4Addr::new(addresses[8], addresses[9], addresses[10], addresses[11]),
    ])
}

/// Negotiated DNS servers, split by address family.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DnsServers {
    /// IPv4 servers.
    pub v4: Vec<Ipv4Addr>,
    /// IPv6 servers.
    pub v6: Vec<Ipv6Addr>,
}

/// Decode `UtaMsCallPsGetNegotiatedDnsRspCb`.
pub fn parse_dns(body: &[u8]) -> Result<DnsServers, CodecError> {
    let format = format!("n{}nsnnnn", "sn".repeat(16));
    let values = codec::unpack(&format, body)?;

    let mut servers = DnsServers::default();
    for i in 0..16 {
        let Some(value) = values.get(2 * i + 1) else {
            break;
        };
        let Some(kind) = values.get(2 * i + 2).and_then(Value::as_int) else {
            break;
        };
        match (kind, value.as_bytes()) {
            (1, Some(bytes)) if bytes.len() >= 4 => {
                servers
                    .v4
                    .push(Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]));
            }
            (2, Some(bytes)) if bytes.len() >= 16 => {
                let mut octets = [0u8; 16];
                octets.copy_from_slice(&bytes[..16]);
                servers.v6.push(Ipv6Addr::from(octets));
            }
            _ => {}
        }
    }
    Ok(servers)
}

/// Decode `UtaSysGetInfoRspCb` into the requested string field.
pub fn parse_sys_info(body: &[u8]) -> Result<Vec<u8>, CodecError> {
    let values = codec::unpack("nns", body)?;
    values
        .last()
        .and_then(Value::as_bytes)
        .map(<[u8]>::to_vec)
        .ok_or(CodecError::MissingArgument)
}
