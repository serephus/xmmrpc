// SPDX-License-Identifier: GPL-2.0 OR BSD-3-Clause
//! Userspace RPC control for the Intel XMM7360 (Fibocom L850-GL) modem.
//!
//! The in-tree `iosm` driver exposes the modem's RPC channel as a WWAN control
//! port. This crate speaks the firmware's reverse-engineered RPC protocol over
//! that port to configure an LTE data connection.

pub mod apn;
pub mod call_ids;
pub mod codec;
pub mod config;
pub mod error;
pub mod modem;
pub mod net;
pub mod requests;
pub mod transport;
pub mod unsolicited;

pub use error::{Error, Result};
