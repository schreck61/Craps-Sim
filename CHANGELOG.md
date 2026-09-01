# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.5.3] - 2026-09-01

Closes the audit 0.5.2 left open: Explorer and Duel had not been checked
against a strategy run.

### Fixed

- **The Explorer called a written strategy flat.** A strategy is entered as
  four rows pinned to `Progression::Flat` — deliberately, because the
  Explorer does not cross a written strategy with the twelve progressions,
  and the engine has a test holding it to that. The pin is a placeholder, not
  a reading, and four places rendered it: the dot tooltip, the results table,
  the copied text, and the dot's ring, which encodes pressing class and drew
  nothing at all — the legend's word for flat. A strategy that Martingales
  its don't pass was described, four ways, as pressing nothing. It was the
  same sentence the Findings histogram carried until 0.5.2, in a screen the
  0.5.2 notes listed as unaudited.

  The three text sites now ask the strategy, and say "as written" when the
  compiled program in hand cannot be proved to be the one that produced the
  rows. The ring is classified from what the strategy declares, ringing for
  the heaviest system it declares: three classes cannot say more than that,
  and "flat" was the one answer that was wrong.

### Audited, no change

- **Duel was already correct.** It hands an authored side an explicitly empty
  selection and the compiled program, omits the progression from the combo
  name rather than printing a placeholder, and reports no per-dollar edge
  because a rule set has no closed form — which is the spec's own rule about
  not drawing a fabricated analytic line.
- Neither screen reads the bet rail's selection at all, which was the defect
  behind all six of the 0.5.2 fixes.

### Added

- `Program::declared_pressing`, the structured form behind
  `Program::pressing_label`, for callers that classify rather than print.

## [0.5.2] - 2026-08-31

### Fixed

Six readouts described the bet rail while a strategy held the dice. None of
them failed — the rail is a real player with real numbers, so each one
answered confidently about somebody who was not playing.

- **The wealth fan drew one flat line at the buy-in.** It traced the bet
  rail, and a strategy run leaves the rail with no live bets of its own, so
  what it drew was a player who never bets: 200 of 200 traced sessions dead
  flat. The engine had no strategy-aware trace at all. `trace_program_wealth`
  is the compiled-player twin of `trace_wealth`, seeded and constructed
  exactly as `run_program_session` so a traced session is the session the
  sweep ran, and the fan now traces whoever is playing.

- **The house edge went missing, then reported the pass line's.** A strategy
  has no closed form — its stakes are conditional, so there is no flat-rate
  blend to solve — and the closed-form call returns nothing without a
  selection. The first fix guarded on whether the rail had bets ticked, which
  is the wrong question: the rail keeps its selection while a strategy plays,
  because switching to Rules must not discard what was on Checkboxes. So with
  the default pass line still on, Findings reported −1.41% per resolved
  dollar — the pass line's edge — for a run the pass line took no part in.
  `SimConfig::closed_form_applies` now names the condition once, and the edge
  is measured instead where no closed form exists: drift over handle, labelled
  as measured and over how many sessions, since a counted edge carries
  sampling error and a solved one does not.

- **The House Line asserted a strategy broke even.** Guarded on flat stakes
  and no quit target, but not on there being a rail at all. With none the
  drift solves to zero and the line was drawn at exactly the buy-in and
  called an expectation; with the rail left as Checkboxes had it, it was
  drawn at the *rail's* expectation, which is worse for looking like a real
  number. It also fed the caption beneath it, which then reported the distance
  from the strategy's mean to a line belonging to a player who never bet.

- **The Anchor offered thirteen pressing systems to a strategy that declares
  its own.** Choosing one re-simulated the empty rail under a progression
  nobody had picked and drew it as this strategy pressed differently. It is
  hidden when a strategy is playing.

- **The histogram was captioned "Flat (no press)" over every strategy.** That
  was the rail's progression, Flat by default. `Program::pressing_label`
  answers from the strategy's own `press` declarations instead, and tells
  apart the three cases worth distinguishing: none declared, one system (and
  whether it covers every stream or only some bets), or several, which no
  single phrase should pretend to summarize. It reports declarations only —
  a strategy can also press from its rules, and no phrase over a histogram
  summarizes a conditional ladder honestly — so the flat case reads "no press
  declared" rather than "no press", which would be false of exactly that
  strategy.

- **The status bar said "no bets selected" when a strategy was playing.**
  That strategy has an edge; it simply is not solvable from the rail. It now
  says so and points at the measured one.

### Changed

- **The bankroll, the table minimums and the house rules are on the Rules tab
  too.** Picking Rules chose the strategy and hid all fifteen run-scope
  settings, because they hung off the bet rail's branch of the same
  conditional. None of them belongs to the rail — a strategy is simulated
  against exactly the same bankroll and the same minimums, and
  `place_the_point` is a table rule deciding what a strategy may legally bet
  that was reachable only while the strategy was not playing. The cost was
  not a missing control but a forced trip across the player switch to reach
  it, which leaves you on the tab where Run means the rail plays.

- **The compiled-strategy performance tripwires are set from a distribution
  rather than from the last reading.** They fired on a commit that added a
  PDF and changed no engine code — the second time the gate went red on a
  machine rather than on a change. Six CI runs over one unchanged engine read
  3.72× to 4.76× on the loaded configurations against a 4.5× tripwire, with
  the small-strategy one six percent from firing next. The ratio is not
  machine-invariant and no number of repetitions makes it so: the runner's own
  speed moves 35% across those runs, and the interpreted side loses more to a
  weak cache than a straight line does, so the ratio tracks the machine. Both
  now sit about four standard deviations above the mean, which leaves a
  catastrophe detector — an interpreter three times slower still trips it —
  and gives up detecting drift of a few percent, which nothing measured on a
  shared runner can do honestly. The table is published to the run summary on
  every run so the trend stays readable.

### Known gaps

- Only Findings and Replay were audited for this. Explorer and Duel read the
  same configuration and have not been checked against a strategy run.
- `Program::pressing_label` reports `press` declarations, not presses driven
  by rules.

## [0.5.1] - 2026-08-30

### Fixed

Three defects found by reading the shipped examples back after 0.5.0 — which
is the right place to have looked first, since an example is where somebody
meets this language.

- **A pressed place bet climbed without limit.** "Press twice, then collect"
  presses on a number's first two hits and regresses after; instead the stake
  ratcheted past $384. Making a decision-point press survive the next
  resolution had been done by writing the rule's figure into the bet's
  pressing stream, and that level then outlived the bet it belonged to: a
  seven-out took the bet, the next roll put a fresh one up at the base stake,
  and resolution topped it straight back to the stale level — from which the
  rule pressed again. A flat stream does not re-price a winner at all now,
  which is what "flat" meant before anyone wrote it down. Only strategies
  that press from a rule were affected; the checkbox player and every
  progression are unchanged, and the pinned outcomes confirm it.
- **"The field is due" bet after one field number, not two.** A decision
  point comes before every roll including the session's first, where the last
  total reads 0 — and 0 is under 4, so a condition written as
  `last-total <= 4` counted a field number that had not happened. The
  superstition was being modelled one roll more eagerly than the superstition
  itself, in the example whose whole job is to be faithful to a belief so the
  app can refute it.
- **A condition shown as text named memory by its slot index.** A condition
  the typed rows cannot take apart is displayed as its own text — and that
  fallback was rendering `var0 >= 2` where the strategy says `streak >= 2`,
  naming something that appears nowhere in the strategy. The statement rows
  directly above it had the names all along.
- A refused `regress` reported itself as a refused bet in the ledger.

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
- **The table answers for itself.** `buy-in`, `table-min` and `table-max`
  are readable, so a stop-loss can be half the buy-in rather than a dollar
  figure that is right at one table and quietly wrong at every other, and a
  Martingale can see its own ceiling coming instead of pushing into a bet
  the table will truncate. Seven of the twelve strategies written against
  this language in review hit this.
- **`press place 6 by 1 unit`** — "press it" is a step, not a destination,
  and computing the destination by hand was the language asking the author
  to do the table's arithmetic. `regress … by` too, and `1 unit` now reads
  as well as `2 units`.
- **Working on the come-out.** `working place 6 on come-out` — the other half
  of a question craps has always had two halves to, and the language could
  only ask one of. A bet is working during a point cycle unless it is called
  off, and is *not* working on the come-out unless it is called on; the second
  half was accepted, acknowledged, and then ignored by the table, so the bet
  sat through the one roll its author had said it should work. The rule
  editor gets a matching slot.
- **The table answers for its own rules.** `field-12-triple` and
  `come-odds-work-on-comeout` are readable, so a strategy whose arithmetic
  assumes a layout can decline a table that does not have it instead of
  playing on and reporting the numbers as though it had.
- **The point may be placed**, where the table allows it — a real one usually
  will, and this engine refused it from the start. Off by default, because
  turning it on changes what every strategy that places numbers does; it
  travels in the Scenario Sentence and in the fingerprint like every other
  house rule.
- **`working-on-come-out(place 6)`**, beside the statement that sets it. A
  language that can say a thing and not ask it back is how the first working
  asymmetry went unnoticed for two revisions.
- **`working(place 6)`** and the **`everything`** group — both were in the
  specification and neither was built. A strategy could turn a bet off and
  not ask whether it had; `down everything` sweeps what a dealer would.
- **`for each of 6, 8 as n with 8, 6 as other { … }`** — lists walked in
  step, so a block can say something about a pair. The 6 and the 8 are
  partners in half the strategies anybody writes, and "when this one wins,
  take the other one down" previously had to be written out per number.
- **`for each of 4, 5, 6, 8, 9, 10 as n { … }`** — a rule written once and
  produced per number. 3-Point Molly was nine rules, six of them the same
  rule with a different number in it; it is four now. Blocks nest, and the
  binding reaches anywhere a number can go. The editor draws a block as one
  card and can unfold it; looking changes nothing, and the block stops being
  a block only when two of its iterations are actually edited apart.

### Fixed

- **The static checks §9 promised.** One of the six shipped; the other five
  were a heading. A rule whose condition can never hold, two unconditional
  rules fighting over one bet, the worst money this strategy can have on the
  layout against the budget, the step at which a pressing system meets the
  table maximum, and the instructions a decision costs — all of them are
  sentences in the order-ticket strip now, before a run rather than after.
  None of them stops a run: a dead rule is legal, and a language that
  refused unsound play could not refute it. The dead-rule check is not a
  theorem prover and says so.
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
- **The version header admits the past and refuses the future.** It refused
  both, which faced the next release with a choice between turning away every
  file on every user's disk and never bumping the number at all — which is
  how a version gate becomes decorative. Every grammar change so far has been
  additive, and additive changes leave old files meaning what they meant. A
  breaking one bumps the number, and old files are migrated or refused
  deliberately.
- **A skipped decision is a decision where no rule fires**, and fire counts
  are the dead-rule diagnostic rather than an implementation detail. The
  compiled player's placement-only skip — which could never engage, because
  nothing on that path cleared the flag it tested — is removed rather than
  repaired: it would have quietly halved the count beside a rule that was
  working perfectly well, on the simplest strategies, which are the ones
  already fast enough.
- **The compiled player got faster where it was slowest.** A bare
  `point != 0` — the commonest single condition in the language, and four
  rules of the loaded benchmark — had no arm in the guard fuser and compiled
  to general stack code beside a fast-path variant nothing constructed. It
  fuses into one test now: measured as a paired A/B on an idle machine, the
  loaded configurations run ~9% faster per roll (219 → 199 ns, 228 → 208 ns).
  The one-rule and nine-rule configurations end up ~1% slower, which is
  everything else this release added to the path every strategy walks —
  worth saying in the same breath as the gain. The built-in player's figures do not move,
  which is the number that matters — the checkbox player pays nothing for a
  language it does not speak.
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
