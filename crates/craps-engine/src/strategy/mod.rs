// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The player, as a thing the table consults.
//!
//! Today the engine *is* the strategy: [`place_bets`](crate::game) hardcodes
//! the policy and [`BetSelection`](crate::BetSelection) only picks which of
//! its branches run. This module inverts that. A strategy proposes
//! [`Action`]s; the table adjudicates them. The built-in player is the first
//! strategy expressed that way, and it must decide exactly what the
//! hand-written version decided, to the cent — the pinned outcome vectors
//! and the equivalence battery are the proof.
//!
//! Design and roadmap: [`STRATEGY_DSL.md`](../../../docs/STRATEGY_DSL.md).

pub mod action;

pub use action::{Action, Adjudication, Amount, BetRef, RejectReason};
