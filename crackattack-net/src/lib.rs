//! Crack Attack! network multiplayer.
//!
//! This crate implements the TCP transport, protocol, and
//! NetworkOpponent for online VS mode. The core crate stays
//! pure (no IO) — all socket work lives here.

// Internal wire format: nothing outside this crate uses it.
pub(crate) mod protocol;
pub mod connection;
pub mod communicator;
pub mod error;
pub mod opponent;
pub mod session;
pub mod discovery;
