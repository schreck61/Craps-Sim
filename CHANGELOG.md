# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
