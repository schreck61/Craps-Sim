# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0] - 2026-08-23

### Added

- macOS releases ship as a signed, notarized, and stapled `Craps-Sim.app`
  in a drag-to-Applications DMG; Gatekeeper accepts first launch without
  warnings. Builds without signing secrets (e.g. forks) fall back to the
  unsigned tar.gz.
- A roll-by-roll session trace harness for inspecting individual sessions
  (`TRACE_MIN`/`TRACE_SEEDS` with the ignored `trace_session` test).

## [0.2.0] - 2026-08-22

### Changed

- The session-length and ending-bankroll phases now share one simulation
  pass per session (the horizon view is a snapshot of the ruin trajectory),
  cutting standard-run work by a third. Proven equivalent to the previous
  split phases seed-by-seed against reference implementations.
- The strategy explorer uses common random numbers: every combo plays the
  same dice sequences per table minimum, so head-to-head rankings are much
  tighter at the same session count.
- The pre-roll placement pass is skipped on rolls that resolved nothing —
  placement only reruns after a resolution changes cash, bets, or the
  point. Proven equivalent against always-place reference implementations.
- Dice pairs are batch-sampled ten 6-bit chunks at a time from each
  generator output (rejection-mapped onto the 36 outcomes), cutting RNG
  calls roughly 2.5x. New statistical tests (chi-square uniformity, serial
  correlation) verify the sampler; dice sequences per seed differ from
  earlier versions, so individual run numbers shift within their
  confidence intervals.

## [0.1.0] - 2026-08-21

Initial release.

### Added

- Massively parallel Monte Carlo craps engine (rayon, integer-cent exact
  payouts, deterministic per-seed sessions).
- Bets: pass line, don't pass, come and don't come (with concurrency caps),
  take/lay odds (1x-10x and 3-4-5x), field, place 4/5/6/8/9/10, hardways,
  any seven, any craps.
- Payout rule variations: field 12 double/triple, come odds on/off on
  come-out, configurable prop stake and table maximum.
- Twelve bet-pressing progressions with per-stream state: flat, full press,
  half press, press-and-pull, Paroli (3-win), 1-3-2-6, Martingale, Grand
  Martingale, D'Alembert, Reverse D'Alembert, Fibonacci, Oscar's Grind.
- Quit-while-ahead (take-profit) rule at a configurable budget multiple.
- Results: ruin-time distributions with 95% confidence intervals, ending
  bankroll deciles with budget multiples, recommended budget for surviving a
  target session length at a chosen confidence.
- Strategy explorer: 11 classic strategies x 12 progressions x 4 quit rules
  ranked per table minimum under five selectable objectives.
- Verification suite: house-edge convergence tests for every bet family,
  deterministic payout walk-throughs, gambler's-ruin closed-form check, and
  drawdown/ruin boundary equivalence.
