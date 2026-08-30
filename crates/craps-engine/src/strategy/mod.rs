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
pub mod ast;
pub mod bench;
pub mod compile;
pub mod examples;
pub mod library;
pub mod player;
pub mod program;
pub mod text;
pub mod view;

pub use action::{Action, Adjudication, Amount, BetRef, RejectReason};
pub use ast::{AmountExpr, BinOp, Expr, Group, Read, Rule, Stmt, Strategy, Trigger, VarId};
pub use bench::{bench_session, BenchEvent, BenchRoll, BenchTrace};
pub use compile::{compile, CompileError};
pub use examples::EXAMPLES;
pub use library::from_selection;
pub use program::{Decision, Program, StratState};
pub use text::{bet_name, parse, render, render_rule, ParseError, LANGUAGE_VERSION};
pub use view::{AllFeatures, FeatureMask, NoFeatures, TableView};
