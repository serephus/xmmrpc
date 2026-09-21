// SPDX-License-Identifier: GPL-2.0 OR BSD-3-Clause
//! Error type shared by the library.

/// Errors returned by modem bring-up.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An operating-system call failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A firmware payload could not be encoded or decoded.
    #[error("codec error: {0}")]
    Codec(#[from] crate::codec::CodecError),

    /// The firmware responded in a way the protocol did not expect.
    #[error("protocol error: {0}")]
    Protocol(String),

    /// The configured configuration file could not be used.
    #[error("configuration error: {0}")]
    Config(String),

    /// The firmware did not answer within the allowed time.
    #[error("timed out waiting for the firmware to respond")]
    Timeout,

    /// The network refused the packet-data attach.
    #[error("the network refused the packet-data attach (wrong APN or no data allowance)")]
    AttachRefused,

    /// No address was assigned within the configured timeout.
    #[error("no IP address was assigned by the network within the timeout")]
    NoIp,

    /// The requested WWAN interface does not exist.
    #[error("network interface {0} does not exist")]
    InterfaceNotFound(String),

    /// A netlink request failed.
    #[error("netlink error: {0}")]
    Netlink(String),

    /// The modem could not be FCC-unlocked.
    #[error("FCC unlock failed")]
    FccUnlock,
}

/// Convenience alias.
pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    /// The process exit code this error maps to.
    ///
    /// Attach refusals and missing addresses are ordinary "no connectivity"
    /// conditions (exit code 2); everything else is a hard failure (exit code 1).
    pub fn exit_code(&self) -> u8 {
        match self {
            Error::AttachRefused | Error::NoIp => 2,
            _ => 1,
        }
    }

    /// Build a netlink error from any displayable error.
    pub(crate) fn netlink(source: impl std::fmt::Display) -> Self {
        Error::Netlink(source.to_string())
    }
}
