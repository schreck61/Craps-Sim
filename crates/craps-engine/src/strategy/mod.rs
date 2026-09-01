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

// The curated `pub use` list below is the surface; the modules behind it are
// not. `Op`, `Guard`, `TriggerTest` and the proposal buffer are interpreter
// internals whose shape is load-bearing — the program identity hash is taken
// over them — and nothing outside this crate has business naming them.
pub(crate) mod action;
pub(crate) mod ast;
pub(crate) mod bench;
pub(crate) mod check;
pub(crate) mod compile;
pub(crate) mod examples;
pub(crate) mod library;
pub(crate) mod player;
pub(crate) mod program;
pub(crate) mod text;
pub(crate) mod view;

pub use action::{Action, Adjudication, Amount, BetRef, RejectReason};
pub use ast::{
    AmountExpr, BinOp, Block, Expr, Group, Read, Rule, Stmt, Strategy, Trigger, VarId, WorkingWhen,
};
pub use bench::{bench_session, BenchEvent, BenchRoll, BenchTrace};
pub use check::{against_table, check, Diagnostic};
pub use compile::{compile, CompileError};
pub use examples::EXAMPLES;
pub use library::from_selection;
pub use program::{Decision, PressClass, Pressing, Program, StratState};
pub use text::{
    bet_name, block_holds, parse, prune_blocks, render, render_rule, ParseError, LANGUAGE_VERSION,
};
pub use view::{AllFeatures, FeatureMask, NoFeatures, TableView};
