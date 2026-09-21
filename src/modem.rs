// SPDX-License-Identifier: GPL-2.0 OR BSD-3-Clause
//! High-level modem bring-up sequence.

use std::net::Ipv4Addr;
use std::thread;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tracing::{debug, info};

use crate::apn;
use crate::call_ids::CallId;
use crate::codec::{self, Arg, Value};
use crate::error::{Error, Result};
use crate::requests::{self, DnsServers};
use crate::transport::{Mode, Rpc};
use crate::unsolicited;

const UTA_MS_SIM_OPEN_REQ: CallId = CallId::new(0x001);
const UTA_MS_SMS_INIT: CallId = CallId::new(0x030);
const UTA_MS_CBS_INIT: CallId = CallId::new(0x025);
const UTA_MS_NET_OPEN: CallId = CallId::new(0x053);
const UTA_MS_CALL_CS_INIT: CallId = CallId::new(0x024);
const UTA_MS_CALL_PS_INITIALIZE: CallId = CallId::new(0x03a);
const UTA_MS_SS_INIT: CallId = CallId::new(0x026);
const UTA_MS_CALL_PS_ATTACH_APN_CONFIG_REQ: CallId = CallId::new(0x1af);
const UTA_MS_NET_ATTACH_REQ: CallId = CallId::new(0x05c);
const UTA_MS_CALL_PS_GET_NEG_IP_ADDR_REQ: CallId = CallId::new(0x049);
const UTA_MS_CALL_PS_GET_NEGOTIATED_DNS_REQ: CallId = CallId::new(0x047);
const UTA_MS_CALL_PS_CONNECT_REQ: CallId = CallId::new(0x051);
const UTA_RPC_PS_CONNECT_TO_DATACHANNEL_REQ: CallId = CallId::new(0x07e);
const UTA_RPC_PS_CONNECT_SETUP_REQ: CallId = CallId::new(0x07d);
const UTA_MODE_SET_REQ: CallId = CallId::new(0x12f);
const UTA_SYS_GET_INFO: CallId = CallId::new(0x07c);
const CSI_FCC_LOCK_QUERY_REQ: CallId = CallId::new(0x18e);
const CSI_FCC_LOCK_GEN_CHALLENGE_REQ: CallId = CallId::new(0x190);
const CSI_FCC_LOCK_VER_CHALLENGE_REQ: CallId = CallId::new(0x192);

/// FCC unlock key read from `nvm:fix_cat_fcclock.fcclock_hash[0]` on the modem.
const FCC_UNLOCK_KEY: [u8; 4] = [0x3d, 0xf8, 0xc7, 0x19];

/// Total time to wait for the modem firmware to start answering RPC requests.
const READY_TIMEOUT: Duration = Duration::from_secs(120);
/// Per-attempt timeout while probing for a ready firmware.
const READY_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Wait until the modem firmware answers a harmless query.
///
/// On a cold boot the `wwan0xmmrpc0` device node appears before the firmware
/// RPC server is ready, so the first request can go unanswered forever. Probe
/// with a short timeout and reset the channel between attempts until the modem
/// responds (or `READY_TIMEOUT` elapses).
pub fn wait_ready(rpc: &mut Rpc) -> Result<()> {
    let deadline = Instant::now() + READY_TIMEOUT;
    loop {
        match rpc.execute_with_timeout(
            CSI_FCC_LOCK_QUERY_REQ,
            &codec::pack_u32(0),
            Mode::Async,
            READY_PROBE_TIMEOUT,
        ) {
            Ok(_) => {
                info!("modem firmware is ready");
                return Ok(());
            }
            Err(Error::Timeout) => {
                if Instant::now() >= deadline {
                    return Err(Error::Timeout);
                }
                debug!("modem is not ready yet, resetting the RPC channel");
                rpc.reopen()?;
            }
            Err(error) => return Err(error),
        }
    }
}

/// Initialise every firmware subsystem this tool needs.
pub fn initialize(rpc: &mut Rpc) -> Result<()> {
    let commands = [
        UTA_MS_SMS_INIT,
        UTA_MS_CBS_INIT,
        UTA_MS_NET_OPEN,
        UTA_MS_CALL_CS_INIT,
        UTA_MS_CALL_PS_INITIALIZE,
        UTA_MS_SS_INIT,
        UTA_MS_SIM_OPEN_REQ,
    ];
    for command in commands {
        rpc.execute(command, &codec::pack_u32(0), Mode::Sync)?;
    }
    Ok(())
}

/// Query the firmware version string.
pub fn firmware_version(rpc: &mut Rpc) -> Result<String> {
    let response = rpc.execute(UTA_SYS_GET_INFO, &requests::sys_get_info(0), Mode::Sync)?;
    let bytes = requests::parse_sys_info(&response.body)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Unlock the FCC radio lock if the modem is locked.
pub fn fcc_unlock(rpc: &mut Rpc) -> Result<()> {
    let response = rpc.execute(CSI_FCC_LOCK_QUERY_REQ, &codec::pack_u32(0), Mode::Async)?;
    let values = codec::unpack("nnn", &response.body)?;
    let state = int_at(&values, 1)?;
    let mode = int_at(&values, 2)?;
    info!(state, mode, "FCC lock status");
    if mode == 0 || state != 0 {
        return Ok(());
    }

    let response = rpc.execute(
        CSI_FCC_LOCK_GEN_CHALLENGE_REQ,
        &codec::pack_u32(0),
        Mode::Async,
    )?;
    let challenge = int_at(&codec::unpack("nn", &response.body)?, 1)?;

    let mut hasher = Sha256::new();
    hasher.update(challenge.to_le_bytes());
    hasher.update(FCC_UNLOCK_KEY);
    let digest = hasher.finalize();
    let answer = u32::from_le_bytes(digest[..4].try_into().unwrap());

    let response = rpc.execute(
        CSI_FCC_LOCK_VER_CHALLENGE_REQ,
        &codec::pack_u32(answer),
        Mode::Async,
    )?;
    if int_at(&codec::unpack("n", &response.body)?, 0)? != 1 {
        return Err(Error::FccUnlock);
    }
    Ok(())
}

/// Put the modem into operating mode `mode` (1 disables airplane mode).
pub fn set_mode(rpc: &mut Rpc, mode: u32) -> Result<()> {
    const MODE_TID: u32 = 15;

    let body = codec::pack("LLL", &[Arg::Int(0), Arg::Int(MODE_TID), Arg::Int(mode)])?;
    let response = rpc.execute(UTA_MODE_SET_REQ, &body, Mode::Sync)?;
    if int_at(&response.content, 0)? != 0 {
        return Err(Error::Protocol(
            "UtaModeSet was rejected by the firmware".into(),
        ));
    }

    loop {
        let message = rpc.recv()?;
        if message.code == unsolicited::UTA_MODE_SET_RSP_CB {
            if int_at(&message.content, 0)? != mode {
                return Err(Error::Protocol(
                    "UtaModeSet could not set the mode (FCC lock enabled?)".into(),
                ));
            }
            return Ok(());
        }
    }
}

/// Configure the APN and request a packet-data attach.
///
/// Returns `false` when the network refuses the attach.
pub fn attach(rpc: &mut Rpc, apn: &str) -> Result<bool> {
    let config = apn::attach_config(apn)?;
    rpc.execute(UTA_MS_CALL_PS_ATTACH_APN_CONFIG_REQ, &config, Mode::Async)?;

    let mut status = attach_status(rpc)?;
    if status == u32::MAX {
        info!("attach refused, waiting for the network to allow attach");
        while !rpc.attach_allowed() {
            rpc.recv()?;
        }
        status = attach_status(rpc)?;
    }

    Ok(status != u32::MAX)
}

fn attach_status(rpc: &mut Rpc) -> Result<u32> {
    let response = rpc.execute(UTA_MS_NET_ATTACH_REQ, &requests::net_attach(), Mode::Async)?;
    int_at(&codec::unpack("nn", &response.body)?, 1)
}

/// Poll the firmware for the negotiated IP address and DNS servers.
pub fn query_ip(rpc: &mut Rpc) -> Result<Option<(Ipv4Addr, DnsServers)>> {
    let response = rpc.execute(
        UTA_MS_CALL_PS_GET_NEG_IP_ADDR_REQ,
        &requests::get_neg_ip(),
        Mode::Async,
    )?;
    let addresses = requests::parse_neg_ip(&response.body)?;

    let response = rpc.execute(
        UTA_MS_CALL_PS_GET_NEGOTIATED_DNS_REQ,
        &requests::get_neg_dns(),
        Mode::Async,
    )?;
    let dns = requests::parse_dns(&response.body)?;

    // On IPv6 networks the first (or first two) slots hold IPv6 bytes; the real
    // IPv4 address is the last non-zero candidate.
    for address in addresses.iter().rev() {
        if *address != Ipv4Addr::UNSPECIFIED {
            return Ok(Some((*address, dns)));
        }
    }
    Ok(None)
}

/// Repeatedly poll for an address until `timeout` elapses.
pub fn wait_for_ip(
    rpc: &mut Rpc,
    timeout: Duration,
    interval: Duration,
) -> Result<Option<(Ipv4Addr, DnsServers)>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(found) = query_ip(rpc)? {
            return Ok(Some(found));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        info!(
            seconds = interval.as_secs(),
            "IP address not available yet, retrying"
        );
        thread::sleep(interval);
    }
}

/// Open the packet-switched data channel.
pub fn open_data_channel(rpc: &mut Rpc, path: &str) -> Result<()> {
    let ps_connect = rpc.execute(
        UTA_MS_CALL_PS_CONNECT_REQ,
        &requests::ps_connect(),
        Mode::Async,
    )?;
    let datachannel = rpc.execute(
        UTA_RPC_PS_CONNECT_TO_DATACHANNEL_REQ,
        &requests::connect_to_datachannel(path)?,
        Mode::Sync,
    )?;

    if ps_connect.body.len() < 6 {
        return Err(Error::Protocol(
            "packet-switched connect response is too short".into(),
        ));
    }
    let mut setup = ps_connect.body[..ps_connect.body.len() - 6].to_vec();
    setup.extend_from_slice(&datachannel.body);
    setup.extend_from_slice(&codec::asn_int(0));
    rpc.execute(UTA_RPC_PS_CONNECT_SETUP_REQ, &setup, Mode::Sync)?;
    Ok(())
}

fn int_at(values: &[Value], index: usize) -> Result<u32> {
    values
        .get(index)
        .and_then(Value::as_int)
        .ok_or_else(|| Error::Protocol(format!("expected an integer at position {index}")))
}
