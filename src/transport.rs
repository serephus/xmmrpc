// SPDX-License-Identifier: GPL-2.0 OR BSD-3-Clause
//! Blocking transport over the `iosm` WWAN RPC control port.
//!
//! The in-tree `iosm` driver exposes the XMM7360 RPC channel as a character
//! device (by default `/dev/wwan0xmmrpc0`). Each read returns one framed
//! message; this module handles framing, request/response correlation and the
//! small amount of state the protocol requires.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tracing::{debug, trace, warn};

use crate::call_ids::CallId;
use crate::codec;
use crate::error::{Error, Result};
use crate::unsolicited;

/// Transaction word used for synchronous requests.
const TXID_SYNC: u32 = 0x1100_0100;
/// Transaction word used for asynchronous requests.
const TXID_ASYNC: u32 = 0x1100_0101;
/// Upper bound on a single read, matching the reference implementation.
const READ_BUF_LEN: usize = 128 * 1024;
/// Sanity bound on a framed message (the largest real message is a few KiB).
const MAX_MESSAGE_LEN: usize = 1024 * 1024;
/// Default time to wait for a response before treating the modem as unresponsive.
pub const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

/// Whether an RPC request is synchronous or asynchronous.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Wait for the direct response only.
    Sync,
    /// Asynchronous request; acknowledgements and indications may arrive first.
    Async,
}

/// The kind of an incoming firmware message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    /// A direct response to a request.
    Response,
    /// An asynchronous acknowledgement (code >= 2000).
    AsyncAck,
    /// An unsolicited indication.
    Unsolicited,
}

/// A decoded firmware message.
#[derive(Debug)]
pub struct Message {
    /// Transaction id word from the header.
    pub tid: u32,
    /// Classification of the message.
    pub kind: MessageKind,
    /// Command or indication code.
    pub code: u32,
    /// Payload body (with the echoed transaction id stripped for async replies).
    pub body: Vec<u8>,
    /// Decoded payload values.
    pub content: Vec<codec::Value>,
}

impl Message {
    fn parse(raw: &[u8]) -> Result<Self> {
        if raw.len() < 20 {
            return Err(Error::Protocol(format!(
                "message too short: {} bytes",
                raw.len()
            )));
        }

        let len_le = u32::from_le_bytes(raw[0..4].try_into().unwrap());
        let mut reader = codec::Reader::new(&raw[4..]);
        let len_asn = reader.take_asn_int()?;
        let code = reader.take_asn_int()?;
        if len_le != len_asn {
            warn!(len_le, len_asn, "rpc length mismatch, framing error?");
        }

        let tid = u32::from_be_bytes(raw[16..20].try_into().unwrap());
        let body_full = &raw[20..];
        let mut content = codec::decode_values(body_full)?;

        let (kind, body) = if tid == TXID_SYNC {
            (MessageKind::Response, body_full.to_vec())
        } else if tid & 0xffff_ff00 == TXID_SYNC {
            if code >= 2000 {
                (MessageKind::AsyncAck, body_full.to_vec())
            } else {
                let echoed = content.first().and_then(codec::Value::as_int);
                if echoed != Some(tid) {
                    return Err(Error::Protocol(format!(
                        "async response is missing the echoed id {tid:#x}"
                    )));
                }
                content.remove(0);
                (MessageKind::Response, body_full[6..].to_vec())
            }
        } else {
            (MessageKind::Unsolicited, body_full.to_vec())
        };

        Ok(Message {
            tid,
            kind,
            code,
            body,
            content,
        })
    }
}

/// An open RPC control port.
pub struct Rpc {
    path: PathBuf,
    file: File,
    inbuf: Vec<u8>,
    attach_allowed: bool,
}

impl Rpc {
    /// Open an RPC control port.
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            path: path.to_path_buf(),
            file: open_port(path)?,
            inbuf: Vec::new(),
            attach_allowed: false,
        })
    }

    /// Close and reopen the port, discarding any buffered or in-flight state.
    ///
    /// The `iosm` RPC channel can be left half-initialised after a request that
    /// the firmware never answered; reopening gives it a clean slate.
    pub fn reopen(&mut self) -> Result<()> {
        self.file = open_port(&self.path)?;
        self.inbuf.clear();
        self.attach_allowed = false;
        Ok(())
    }

    /// Whether the firmware has indicated that a packet-data attach is allowed.
    pub const fn attach_allowed(&self) -> bool {
        self.attach_allowed
    }

    /// Send `cmd` and wait for its response.
    ///
    /// Asynchronous acknowledgements and unsolicited indications received while
    /// waiting are processed and discarded.
    pub fn execute(&mut self, cmd: CallId, body: &[u8], mode: Mode) -> Result<Message> {
        self.execute_with_timeout(cmd, body, mode, RESPONSE_TIMEOUT)
    }

    /// Like [`execute`](Self::execute) but fails after `timeout`.
    pub fn execute_with_timeout(
        &mut self,
        cmd: CallId,
        body: &[u8],
        mode: Mode,
        timeout: Duration,
    ) -> Result<Message> {
        let mut packet = build_header(cmd, body.len(), mode);
        packet.extend_from_slice(body);

        trace!(cmd = %cmd, bytes = packet.len(), "rpc request");
        self.file.write_all(&packet)?;

        loop {
            let message = self.recv_inner(Some(timeout))?;
            match message.kind {
                MessageKind::Response => return Ok(message),
                MessageKind::AsyncAck => trace!(code = message.code, "rpc async ack"),
                MessageKind::Unsolicited => {
                    trace!(code = message.code, "rpc unsolicited (skipped)")
                }
            }
        }
    }

    /// Read and process a single message.
    pub fn recv(&mut self) -> Result<Message> {
        self.recv_inner(None)
    }

    fn recv_inner(&mut self, timeout: Option<Duration>) -> Result<Message> {
        let raw = self.read_message(timeout)?;
        let message = Message::parse(&raw)?;
        if message.kind == MessageKind::Unsolicited {
            let name = unsolicited::unsolicited_name(message.code).unwrap_or("unknown");
            debug!(code = message.code, name, "rpc unsolicited");
            self.handle_unsolicited(&message);
        }
        Ok(message)
    }

    fn handle_unsolicited(&mut self, message: &Message) {
        if message.code == unsolicited::UTA_MS_NET_IS_ATTACH_ALLOWED_IND_CB
            && let Some(allowed) = message.content.get(2).and_then(codec::Value::as_int)
        {
            self.attach_allowed = allowed != 0;
        }
    }

    /// Read exactly one framed message, buffering any surplus bytes.
    fn read_message(&mut self, timeout: Option<Duration>) -> Result<Vec<u8>> {
        let deadline = timeout.map(|timeout| Instant::now() + timeout);

        loop {
            if self.inbuf.len() >= 4 {
                let total = u32::from_le_bytes(self.inbuf[0..4].try_into().unwrap()) as usize + 4;
                if !(20..=MAX_MESSAGE_LEN).contains(&total) {
                    return Err(Error::Protocol(format!(
                        "invalid RPC frame length {total}, channel out of sync"
                    )));
                }
                if self.inbuf.len() >= total {
                    return Ok(self.inbuf.drain(..total).collect());
                }
            }

            self.wait_readable(deadline)?;

            let mut chunk = [0u8; READ_BUF_LEN];
            let read = self.file.read(&mut chunk)?;
            if read == 0 {
                return Err(Error::Protocol("RPC port closed".into()));
            }
            self.inbuf.extend_from_slice(&chunk[..read]);
        }
    }

    /// Wait until the port has data, or `deadline` passes.
    fn wait_readable(&self, deadline: Option<Instant>) -> Result<()> {
        let Some(deadline) = deadline else {
            return Ok(());
        };

        let now = Instant::now();
        if now >= deadline {
            return Err(Error::Timeout);
        }
        let timeout_ms = (deadline - now).as_millis().min(i32::MAX as u128) as i32;

        let mut pollfd = libc::pollfd {
            fd: self.file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `pollfd` is a valid pointer to a single initialised entry.
        let ready = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };
        if ready < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        if ready == 0 {
            return Err(Error::Timeout);
        }
        Ok(())
    }
}

fn open_port(path: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_SYNC)
        .open(path)?)
}

fn build_header(cmd: CallId, body_len: usize, mode: Mode) -> Vec<u8> {
    let tid = match mode {
        Mode::Sync => 0,
        Mode::Async => TXID_ASYNC,
    };
    let tid_word = TXID_SYNC | tid;
    let total = body_len + 16 + if tid != 0 { 6 } else { 0 };

    let mut header = Vec::with_capacity(26);
    header.extend_from_slice(&(total as u32).to_le_bytes());
    header.extend_from_slice(&codec::asn_int(total as u32));
    header.extend_from_slice(&codec::asn_int(cmd.raw()));
    header.extend_from_slice(&tid_word.to_be_bytes());
    if tid != 0 {
        header.extend_from_slice(&codec::asn_int(tid));
    }
    header
}

#[cfg(test)]
mod tests {
    use std::os::fd::{FromRawFd, OwnedFd};

    use super::*;

    /// Build an `Rpc` backed by the read end of a pipe.
    fn rpc_from_pipe() -> (Rpc, OwnedFd) {
        let mut fds = [0i32; 2];
        // SAFETY: `fds` is a valid two-element array.
        let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
        assert_eq!(rc, 0, "pipe2 failed");
        // SAFETY: `pipe2` returned two owned file descriptors.
        let (read_end, write_end) =
            unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        let rpc = Rpc {
            path: PathBuf::new(),
            file: File::from(read_end),
            inbuf: Vec::new(),
            attach_allowed: false,
        };
        (rpc, write_end)
    }

    #[test]
    fn read_times_out_on_a_quiet_port() {
        let (mut rpc, _write_end) = rpc_from_pipe();
        let result = rpc.read_message(Some(Duration::from_millis(50)));
        assert!(matches!(result, Err(Error::Timeout)));
    }

    #[test]
    fn read_rejects_an_implausible_frame_length() {
        let (mut rpc, write_end) = rpc_from_pipe();
        let mut fd = File::from(write_end);
        fd.write_all(&u32::MAX.to_le_bytes()).unwrap();
        let result = rpc.read_message(Some(Duration::from_millis(50)));
        assert!(matches!(result, Err(Error::Protocol(_))));
    }
}
