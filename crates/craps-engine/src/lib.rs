// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Craps simulation engine. All money is in integer cents so payouts are exact.
//!
//! Deterministic per seed: every session's dice derive from
//! [`session_seed`], the crate's one seed formula. The engine knows nothing
//! about storage or UI — sweeps stream batches over a channel and stop.

pub mod rng;

mod bets;
mod edge;
mod game;
mod session;
mod stats;
pub mod strategy;
mod summary;
mod sweep;
pub mod trace;

pub use bets::*;
pub use edge::*;
pub use rng::Xoshiro256pp;
pub use session::*;
pub use stats::*;
pub use strategy::{Action, Amount, BetRef, RejectReason};
pub use summary::*;
pub use sweep::*;

/// The engine's version, stamped into run provenance and exports.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
