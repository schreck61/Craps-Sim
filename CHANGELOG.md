# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.5.0] - 2026-08-30

### Added

- **A strategy language.** Betting strategies can now be written, not just
  ticked: `on <trigger> when <condition>: <actions>`, with pressing declared
  per bet stream. What the checkbox player could never say — press the 6 on
  its own second hit, call the place bets off until the shooter makes a
  point, Martingale the don't pass while the place bets stay flat, stop at
  −$200 — is now sayable, runnable, and rankable. Design's **Rules** tab
  builds one from typed slots where a syntax error is not representable, or
  from text; both edit the same tree, and a round-trip law holds them
  together. Four worked examples ship under **Examples**; none of them is
  advice.
- **The Bench**, on Replay: a strategy's night stepped roll by roll, with
  every rule that fired marked, every cent attributed to the rule that asked
  for it or to the table, the session's refusals listed with a way to reach
  each one, and a fire count per rule so a rule that never fires says so.
- **Strategies are saved** as plain-text `.craps` files beside the
  preferences, and the Scenario Sentence carries the one that played by name
  and content hash — `playing "44 Inside, regressed" #9f3c1a2b`. A pasted
  sentence naming a strategy whose rules have changed says so rather than
  running the wrong player.
- **The Explorer ranks your strategy too**, as its own rows beside the
  curated eleven, and the **Duel** will run it against any of them on
  identical dice.
- **`on come point on 6:`** — a rule for a come flat reaching a number,
  which is a different event from the table's point being established and
  previously took a four-rule memory idiom per number to approximate.
- **`paid(place 6)`** — what a bet was just paid, so pressing by half the
  winnings is a rule and not only a progression.
- **`for each of 4, 5, 6, 8, 9, 10 as n { … }`** — a rule written once and
  produced per number. 3-Point Molly was nine rules, six of them the same
  rule with a different number in it; it is four now. Blocks nest, and the
  binding reaches anywhere a number can go. The editor draws a block as one
  card and can unfold it; looking changes nothing, and the block stops being
  a block only when two of its iterations are actually edited apart.

### Fixed

- **A strategy can no longer quietly play something other than what it
  says.** A review of the language ([STRATEGY_DSL_REVIEW.md](docs/STRATEGY_DSL_REVIEW.md))
  found that its worst faults were all silent ones, which is the single
  thing the design set out to prevent:
  - A named stake below the table minimum was accepted. `bet pass 12` — a
    bare number is cents — rode for twelve cents through three hundred rolls
    without one refusal. Sub-minimum stakes are refused now, oversized ones
    are clipped with the event that says so, and money renders back as money
    rather than as the cent count it became.
  - **Presses could not climb.** A progression re-prices a bet where it
    resolves, so a rule pressing at the decision point was undone by the
    very win it was riding, and `press place 6 to stake(place 6) * 2`
    recomputed twice-the-base forever. Every press-and-ride ladder in craps
    was unreachable, including the one the shipped example's own prose
    describes. A rule that names a figure now tells the bet's stream what
    the bet is worth.
  - `on session-start` parsed, compiled, and appeared first in the editor's
    trigger list. Nothing ever set its bit, so every rule written on it did
    nothing. `var mult = 1` parsed an initializer and discarded it. Both are
    honoured now.
  - `place 260` wrapped into a legal place 4 and `total(263)` into a trigger
    that fired on every seven. Out-of-range numbers are refused, and the
    message names the number rather than the word after it.
  - Odds references silently meant the flat behind them in win/loss
    triggers, history reads, and pressing declarations. The grammar no
    longer draws a distinction the engine does not have.
  - A `for each` block whose body touched memory dissolved into copies on
    the first save, comments and all.
  - "Not allowed right now" covered three different rules of craps; it names
    which one now. A press in the wrong direction was the only refusal in
    the language that emitted nothing.
- **Two ways to crash the app from pasted text.** The expression parser
  recursed without a depth limit, so a few thousand parentheses overflowed
  the stack — the test written to prove malformed input is never fatal was
  itself crashing the engine's test suite. And `i64::MIN / -1` panics in
  every build profile, from constants any strategy can write, on every core
  at once. Nesting is bounded and division saturates.
- The Explorer indexed the curated eleven with the sentinel that marks an
  authored strategy — a panic waiting for one to rank. It gets a lane of its
  own, and reports no edge rather than an invented one.
- Pasting a Scenario Sentence could not then be run: validation still asked
  the bet rail for bets, which a strategy sentence deliberately blanks. It
  also opened over unsaved editor work without asking, and could not find a
  strategy whose name contained a character the file store rewrites.
- The odds policy was derived from the bet rail's own `take/lay odds`
  checkbox, so a strategy asking for maximum odds would have been refused at
  any table where that box happened to be unticked.
- Errors are no longer painted in the ink reserved for ruin. A configuration
  that will not run is a problem, not a bust.

### Changed

- The configuration fingerprint now includes which player is live, so
  results go stale when the strategy changes. This is a deliberate format
  change; results computed by earlier builds are not comparable by
  fingerprint.
- `BetEventKind::Won` records whether the stake came back, which a ledger
  needs in order to account for the rail. Place and hardway winners stay up;
  everything else returns.
- **The two proofs the language leans on now run in CI.** The
  ten-thousand-seed equivalence battery — the hand-written player and the
  compiled one pinned to each other on identical dice — and the performance
  budget were both marked ignored and in no job, which left the whole
  argument for keeping two player implementations enforced by nobody. Both
  halves of the budget are asserted; the smaller half is 2.5× rather than
  the 2× first written, because the nine-rule 3-Point Molly has measured
  above 2× since the day it was first benchmarked and a budget contradicted
  by the table printed beside it is not a budget.

## [0.4.3] - 2026-08-24

### Fixed

- Dim widget text on systems whose OS runs in light mode. egui keeps
  separate dark/light style slots and resolves the active one from the OS
  theme, which arrives one frame after startup — the app's colors landed
  in the slot that was about to go inactive, leaving egui's factory
  defaults on screen until the user toggled themes by hand. The theme now
  writes both slots, and a first-frame re-sync means a first launch with
  no saved preference follows the real OS theme.

### Added

- An application icon — the "Seven-Out Curve": the ending-bankroll
  histogram with its ruin-red bust bar and a thrown 4-and-3 landing at
  its foot. Shipped in the macOS bundle, embedded in the Windows
  executable, and set as the runtime window icon everywhere.

## [0.4.2] - 2026-08-24

### Fixed

- A fresh Duel opened its replayed night at roll zero — an empty
  trajectory panel until a session pick was clicked. It now opens fully
  revealed.
- The trajectory chart's "population envelope · sampled n" provenance
  line was drawn on the bottom paint layer, where every wealth track
  painted over it; it now sits above the data on a legible pill.
- Hexbin cells were culled by center only, letting edge hexes overhang
  the axis gutters onto the tick numbers; they are now culled by their
  full extent.

### Changed

- Every label that can sit on data marks now draws on a near-opaque
  ground pill, following an audit of all text-over-chart sites: the
  Explorer strip's hover readout; survival line-end, censor, and probe
  labels; the histogram's log-scale note, house-gap caption, pinned
  edge, brush mass, and tie-verdict banner; trajectory drift, mean,
  end-tick, and series labels ("night #N" and "bust" also no longer
  stack); hexbin quadrant counts; ladder percentile, 1×, and mean labels
  (the mean drops a row when it sits near 1×, and the paired ladder
  skips labels that would crowd); the Stake budget and confidence
  readouts; the ghost panel's copy; and Design's blended-edge label.

## [0.4.1] - 2026-08-24

### Fixed

- Replay's play button plays. The theater still opens with the full night
  revealed; play now rewinds to the top when pressed at the end, resumes
  in place mid-night, drives its own frames (the playhead previously
  advanced only while the mouse moved), and defaults to 60× — 1× is
  literal casino time (one roll every ~36 s) and remains one keypress
  away.
- The Duel re-simulates explorer combos under the explorer's rules, where
  the odds policy is always available to strategies that take odds.
  Previously the main Design's "take odds" toggle could silently strip
  odds from a duel of odds-taking strategies.
- The Duel screen scrolls: the A-vs-B scatter, paired-difference
  histogram, and decile ladder were unreachable below the fold.

### Changed

- Custom table minimums join the Design chip row as real chips in their
  reserve color — a $250 clicks off like any canonical chip; the text
  field remains the way to add one.
- The Explorer gains an always-visible Duel bar: it explains
  click/shift-click selection, names both armed sides in their colors,
  and offers an "Open the Duel ▶" button (D still works). Mismatched
  table minimums are called out instead of silently doing nothing.
- The Duel's session picks highlight the active choice beside a
  "showing night #N" readout.
- In Explorer mode the left rail reports the sweep (progress, session
  count, finish time) instead of the Findings run's per-minimum lanes.

## [0.4.0] - 2026-08-24

### Changed

- The interface is rebuilt (the "Long Run" redesign — the app itself is
  still Craps Sim): one window, four modes —
  Design, Findings, Explorer, and Replay — with each of the four core
  questions on its own screen: Endurance (how long the money lasts),
  Horizon (where the bankroll ends up), and Stake (how big a bankroll you
  need) as Findings tabs, and the Explorer (what to play) as its own mode.
- Runs stream: sessions arrive in batches while the simulation is still
  going, so charts assemble live — bars grow, curves unfurl, confidence
  ribbons visibly narrow — with instant cancellation and partial results
  that stay fully interactive.
- The seed is explicit and user-visible (shown in the rail, click to copy,
  one click to reroll), replacing wall-clock entropy; every chart corner
  carries its provenance (seed, session count, scenario).

### Added

- Closed-form house edge as permanent furniture: per-bet edge ticks and a
  live blended edge strip in Design, a fixed per-resolved-dollar EDGE
  column in the Explorer, and the edge in the status baseline on every
  screen.
- The Anchor: swap any of the twelve progressions on the Horizon histogram
  and watch the distribution morph, re-simulated on identical dice — the
  per-dollar edge marker stays pinned while the expected-loss pin drifts
  with the handle.
- The Duel: paired comparison of two strategies under common random
  numbers — identical dice for both, with a same-dice replayed session, a
  y = x scatter, a paired-difference histogram (tie band when the CI on
  the median difference includes zero), and a paired decile ladder.
- Session Replay theater: any percentile, bar, or dot opens the exact
  re-simulated session — the bankroll trajectory drawn inside the
  population's percentile envelope, above a roll-by-roll dice strip with
  bet resolutions to the cent and full playback transport.
- The Scenario Sentence: the entire configuration rendered as one line of
  prose atop every mode. Copying the sentence is the save format; pasting
  one into Design reconstructs the configuration.
- Chart export: copy (⌘⇧C) or save (⌘⇧S) any chart as a 2× PNG with
  title, Scenario Sentence, seed, and session count baked into the pixels.
- Chart interaction: drag brushes a range and zooms to it, ⌘-scroll zooms
  at the cursor, ⇧-scroll pans (clamped to the data; double-click resets,
  with an on-chart hint while zoomed), and clicking pins up to three
  reference lines that print their axis values, show the Δ between the
  last two, and survive into exports. A new run resets each chart's zoom
  and pins so the view recenters on the fresh distribution.
- Dark and light themes (⌘L), following the OS on first launch, with six
  embedded fonts — renamed OFL 1.1 subsets of Fraunces, Inter, and IBM
  Plex Mono (see `crates/craps-app/assets/fonts/FONTS-NOTICE.md`).
- Engine surface: a public `session_seed` determinism contract routed
  through every consumer, a roll-by-roll session trace API, a
  resolved-handle accumulator (mean loss ≈ edge × handle, verified across
  all twelve progressions), 95% confidence intervals for all five explorer
  objectives, and a batched streaming sweep API with cancellation.

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
