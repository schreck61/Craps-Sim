# The Player

## Craps-Sim Strategy Language — Design Specification & Implementation Plan

**Status:** Proposal — nothing in this document is implemented yet. It targets
the v0.5 release.

The Long Run ([GUI_DESIGN.md](GUI_DESIGN.md)) built the instrument. This
document specifies its subject: a way to express *any* playing strategy —
conditional, stateful, arbitrarily complex — and simulate it at the throughput
the instrument already assumes.

**Part I** is the design specification: the model, the language, the two
editors, the debugging surface, and the honesty rules the design enforces.
**Part II** is the implementation plan: engine surface changes, the compiled
representation and its performance budget, milestone roadmap, testing strategy,
and risk register. Cross-references of the form "GUI spec §N" refer to
[GUI_DESIGN.md](GUI_DESIGN.md); "spec §N" refers to Part I of this document.

---

# Part I — Design Specification

## 1. Where the Ceiling Is

Today a strategy is a flat declarative struct: [`BetSelection`](../crates/craps-engine/src/bets.rs)
(which bets stand) plus one global `Progression`. The engine hardcodes the
policy in [`place_bets`](../crates/craps-engine/src/game.rs) — *"if
`sel.pass_line` and none is up, bet the table minimum."* The strategy is not a
thing the engine consults. The engine **is** the strategy, and `BetSelection`
only selects which of its branches are live.

That single fact is the ceiling. Nothing conditional on history can be said:

- *"Place the 6 and 8; press each on its second hit; take both down after the third."*
- *"Martingale the don't pass, but keep the place bets flat."*
- *"Regress from $44 inside to $22 after the first winner."*
- *"Turn the place bets off after a seven-out until the shooter makes a point."*
- *"Stop at −$200 or +$150, whichever comes first."*
- *"Bet the field only after two field numbers in a row."*

These are the strategies people actually argue about. The last one is nonsense,
and the app exists to show that it is nonsense — which it cannot do while the
nonsense is inexpressible.

## 2. Design Principles

Seven principles govern this document. When a future feature conflicts with
one, the feature loses. They sit under, not beside, the seven in GUI spec §1 —
Principle 6 there ("Every Number Carries Its Provenance") is why §10 exists at
all.

**1. The Table Adjudicates.** A strategy never mutates the layout. It *proposes
intents*; the engine validates every one against point state, table minimum and
maximum, payout-unit rounding, and bankroll, then applies or rejects it. There
is exactly one place where money moves, and it is not in the language.

**2. One AST, Two Editors.** The rule table and the text form are the same
abstract syntax tree serialized two ways. Anything the GUI can build, the text
can express; anything the text expresses, the GUI can render and edit. This is
enforced by a round-trip property test, exactly as the Scenario Sentence codec
is today. The alternative — a click-together builder for toy strategies bolted
to a scripting escape hatch that cannot be visualized — is the failure mode this
principle exists to forbid.

**3. Bounded by Construction.** The language is deliberately not
Turing-complete: no recursion, no unbounded loops, no dynamic allocation, no
I/O. Every strategy is total, terminating, and cost-bounded before it runs.
This is not asceticism. It is what makes it safe to execute an unreviewed,
user-authored strategy 1.2 million times across every core, and what makes
static analysis (§9) possible at all.

**4. Nothing Silent.** A rule that never fires, a bet that never lands, a stake
that clipped against the table maximum, an action rejected for insufficient
bankroll — each is a visible event, in Replay's ledger and in the Bench (§8). A
complex strategy that quietly does nothing and returns a confident distribution
is the single worst outcome this feature can produce, and the design's main job
is to make it impossible.

**5. The Language Must Be Able to Be Wrong.** It must express superstition —
due numbers, hot shooters, hedges that cannot hedge — faithfully and without
editorial. The app does not refuse to model a belief; it models it exactly and
draws what happens. A language that only permits sound play cannot refute
unsound play, and refutation is the product.

**6. Pure and Deterministic.** Strategy evaluation is a pure function of
`(table state, events since the last decision, strategy memory)`. No clock, no
RNG, no ambient state. Replay, the Duel, the Anchor, and every common-random-
numbers guarantee in GUI spec §10 depend on this without exception.

**7. Every Strategy Carries Its Provenance.** A strategy travels as a name plus
a content hash of its compiled form. A pasted Scenario Sentence can never
silently run against a different strategy that happens to share a name.

## 3. The Model

Three pieces, in the engine, under `craps-engine/src/strategy/`.

### 3.1 One decision point

A real player acts between rolls. So there is one hook, called once per roll,
after resolution and before the dice leave the shooter's hand:

```rust
fn decide(&self, view: &TableView, events: &[BetEvent], mem: &mut StratState) -> ActionList
```

"On the point being established", "on a seven-out", "on the 6 hitting" are
**sugar** — conditions over `events`, not additional hooks. One hook is simpler
to specify, simpler to test, and matches what a player can actually do at a
table. Call bets ("in the middle of the roll") are out of scope (§12).

### 3.2 What a strategy may read — `TableView`

A read-only borrow, no allocation. The vocabulary is deliberately generous,
because this is what decides which strategies are expressible:

| Group | Reads |
|---|---|
| Dice & point | `point` (0 = off), `last_total`, `roll`, `rolls_this_shooter`, `shooter` |
| Money | `cash`, `wealth`, `profit`, `peak_profit`, `drawdown`, `handle` |
| Layout | `up(bet)`, `stake(bet)`, `working(bet)`, `live_come`, `live_dont_come`, `on_table_face` |
| History | `hits(n)`, `hits_this_shooter(n)`, `wins(bet)`, `losses(bet)`, `streak(bet)` |
| Memory | user-declared variables and flags |

All values are `i64` cents or counts. **There is no floating-point arithmetic in
the language** — money is integer cents end to end, as everywhere else in this
engine, and a strategy that could compute `$12.333…` would break payout
exactness.

Derived history (`hits`, `streak`, …) is maintained only when a compiled
strategy actually reads it. The compiler emits a feature mask; the session
maintains exactly those accumulators and nothing else, so the default player
pays nothing for vocabulary it never uses (S2).

### 3.3 What a strategy may do — `Action`

```rust
enum Action {
    Bet(BetRef, Amount),          // put it up if it is not up; idempotent
    Odds(BetRef, Amount),         // take or lay behind a line/come point
    Press(BetRef, Delta),         // to X, or by X, out of the winnings
    Regress(BetRef, Delta),
    Down(BetRef),                 // take it down where the table allows
    Working(BetRef, bool),        // off/on — place bets on the come-out
    Set(VarId, Expr),
    Leave(Reason),                // the quit rule, generalized
}
```

`BetRef` names a bet or a **group**: `place inside` (5, 6, 8, 9), `place
outside` (4, 10), `all place`, `all hardways`, `everything`. Groups are sugar,
expanded at compile time.

`Amount` is `Cents(i64) | Units(n)` — *n* table minimums, rounded up to the
bet's payout unit — `| Multiple(n, of)` | `Max` (the odds cap under the
configured policy) `| Half | All`.

Adding a bet type the engine does not yet model (buy, lay, place-to-lose, hop,
horn) is one `BetKind` variant plus its resolution arm. It is not a grammar
change. The language is designed so the layout can grow under it.

### 3.4 Adjudication

The table applies an `ActionList` in order and emits an event per action. New,
in [trace.rs](../crates/craps-engine/src/trace.rs):

```rust
BetEventKind::Rejected { reason: RejectReason }
```

with `RejectReason ∈ { BelowTableMinimum, AboveTableMaximum, NotAllowedNow,
InsufficientBankroll, NoSuchBet, AlreadyDown, ClippedToMax }`. Per Principle 4,
every rejection surfaces. `ClippedToMax` is not an error — it is how the table
maximum truncates a Martingale, and the user must see the truncation rather
than infer it from a flat spot in a curve.

## 4. The Rule

A strategy is a header, an optional memory declaration, and an **ordered list of
rules**:

```
Rule := on <Trigger> [when <Condition>] : <Action>+
```

Rules are evaluated top to bottom at each decision point; matching rules append
their actions to one intent list; the table then applies the list in order. Two
rules touching the same bet are resolved by order — last write wins — and the
Bench shows both firing, so the shadowing is seen rather than debugged.

**Triggers.** `session-start`, `come-out`, `point-established`, `point-made`,
`seven-out`, `roll`, `total(n)`, `win of <bet>`, `loss of <bet>`,
`every n rolls`.

**Conditions.** Any boolean expression over §3.2 reads: comparisons, `and`,
`or`, `not`, `+ - * / min max`, integer only.

**Memory.** `var name = <expr>` declares a session-scoped counter, reset at
session start. Fixed count, resolved to slot indices at compile time; there is
no dynamic namespace.

**Bounded iteration.** `for each of 6, 8 { … }` and `for each place number { … }`
— iteration over a compile-time-known list, and nothing else (Principle 3).

## 5. Two Editors, One AST

### 5.1 The rule table (the friendly surface)

The Design screen (GUI spec §6.1) keeps its bet rail unchanged — the checkbox
player is the beginner surface and does not go away. Beneath it, a new **Rules**
section, empty by default, renders each rule as an English sentence made of
typed slots:

> **When** `the point is established` **→** `place` `6 and 8` for `2 units`
>
> **When** `place 6 wins` **and** `hits on 6 ≥ 2` **→** `press` `place 6` to `$18`
>
> **When** `profit ≥ $150` **→** `leave the table`

Every slot is a dropdown or a validated number field. **Syntax errors are not
representable.** Rules drag to reorder; each row shows a live count of how often
it fired in the last Bench run, which is how a dead rule announces itself
without anyone going looking.

This is the Scenario Sentence idea generalized, and it reuses that machinery
directly: [sentence.rs](../crates/craps-app/src/sentence.rs) is already a
lossless bidirectional codec between configuration and clickable prose
fragments with per-fragment staleness diffing. The rule renderer is the same
`Span`/`FragmentId` pattern applied to a tree instead of a line. Nothing new is
invented alongside it.

### 5.2 The text form (the same tree, typed)

For authors who would rather type, and for pasting a strategy between users:

```
strategy "44 Inside, regressed" language 1

  var hits = 0

  on point-established:
      bet place inside 2u              # $22 inside at a $5 table

  on win of any place:
      set hits = hits + 1
      when hits == 2: regress all place to 1u
      when hits >= 4: down all place

  on seven-out:
      set hits = 0

  on roll when profit <= -20000:
      leave "stop-loss"
```

`render(parse(text)) == text` and `parse(render(ast)) == ast`, both property-
tested over randomized rule sets — the same law the Scenario Sentence already
obeys. The `language 1` header is binding: the parser refuses a version it does
not know rather than guessing, so a saved strategy never silently changes
meaning under a grammar revision.

## 6. Progressions Become Rules

Martingale is not a feature. It is:

```
  on loss of pass: press pass to 2x
  on win of pass:  regress pass to 1u
```

The twelve progressions therefore stop being a separate axis of the engine and
become a **built-in library of rule fragments, attachable per bet stream**. Two
things follow:

1. The Explorer's progression axis (GUI spec §6.6) is preserved exactly — the
   twelve fragments are named, enumerable, and crossable as they are today.
2. Per-stream progressions become expressible for the first time: *Martingale
   the don't pass, flat place bets.* Today `Progression` is one global enum and
   this cannot be said. This also retires the "per-bet Anchor variants beyond
   progressions" item in [v-next.md](v-next.md) by giving it a real
   representation.

The existing [`Progression`](../crates/craps-engine/src/bets.rs) enum and its
`description()` strings survive as the library's labels; the GUI picker and its
sparklines are unchanged.

## 7. Worked Examples

Each is a strategy that cannot be expressed today.

**Press the 6 and 8 twice, then collect.**
```
  on point-established: bet place 6, 8 at 1u
  on win of place(n) when hits_this_shooter(n) <= 2: press place(n) by 1u
  on win of place(n) when hits_this_shooter(n) > 2:  regress place(n) to 1u
```

**Hedged don't with flat place bets** (per-stream progressions):
```
  use martingale on dont-pass
  use flat on all place
  on come-out: bet dont-pass 1u
  on point-established: bet place 6, 8 at 1u
```

**Off after a seven-out until the shooter makes a point.**
```
  var trusted = 0
  on seven-out:  set trusted = 0
  on point-made: set trusted = 1
  on roll when trusted == 0: working all place off
  on roll when trusted == 1: working all place on
```

**Stop-loss and stop-win together** (the current quit rule, generalized):
```
  on roll when profit >= 15000 or profit <= -20000: leave
```

**Superstition, modeled honestly** (Principle 5):
```
  var streak = 0
  on roll when last_total in 2,3,4,9,10,11,12: set streak = streak + 1
  on roll when not (last_total in 2,3,4,9,10,11,12): set streak = 0
  on roll when streak >= 2: bet field 1u
```
The Duel then puts this against flat pass-line on identical dice, and the
paired-difference histogram says the rest.

## 8. The Bench

**This is the feature that decides whether the language is usable, and it is not
optional.** An editor without a debugger is where this kind of feature dies:
complex rule sets fail silently, and Principle 4 is unenforceable by inspection.

The Bench is a panel on the Design screen that runs **one session on a fixed
seed** and steps it roll by roll:

- The dice, the point, and the layout after each roll.
- **Which rules fired**, in order, highlighted in the rule table.
- Every action and its adjudication — applied, clipped, or rejected with its
  reason in words: `place 4 for $5 — rejected: below table minimum ($6 unit)`.
- Cash, wealth, and profit to the cent at every step.
- A per-rule fire count for the whole run, surfaced back onto each rule row.

It reuses Replay's transport and dice strip (GUI spec §6.8) and
[`trace_session`](../crates/craps-engine/src/trace.rs) wholesale; the only new
machinery is rule attribution, which the interpreter emits as an event when the
Bench observer is attached (and never otherwise, per the `Noop` discipline).

## 9. Static Checks

Compile-time, shown in the editor as plain sentences, in the register of GUI
spec §6.1's order-ticket validation:

- **Never bets.** No reachable rule places a bet — Run is disabled and says so.
- **Dead rule.** A rule whose condition is unsatisfiable, or shadowed by an
  earlier rule on every path.
- **Conflict.** Two rules acting on the same bet at the same trigger; ordering
  resolves it, but the user is told which wins.
- **Exposure.** Worst-case outlay on the first shooter against the configured
  budget — the existing exposure strip, computed from the rule set instead of
  the checkbox set.
- **Clipping.** A progression fragment whose step *k* stake exceeds the table
  maximum, in the exact words already used today: `table max $1,000 <
  Martingale step 7 stake $1,280 — the progression will clip`.
- **Cost.** The static instruction-count bound per roll, shown in the Engine
  disclosure. Principle 3 makes this computable; a strategy that would halve
  throughput should say so before it runs, not after.

## 10. Provenance and the Save Format

The Scenario Sentence is the save format and is lossless by contract (GUI spec
§6.1, Principle 6). An arbitrary rule set does not fit in one line of prose, and
pretending otherwise would break the one contract the whole app leans on.

**Decided:** the sentence carries the strategy **by name plus a short content
hash of its compiled form** — `playing "44 Inside, regressed" [9f3c1a]` — with
strategy bodies stored in a side library of plain-text `.craps` files
(§5.2's text form, which is already the serialization). Consequences, all
intended:

- A pasted sentence whose hash does not match the local strategy of that name is
  **STALE**, loudly, in the existing amber register. It never silently runs the
  wrong strategy.
- A sentence whose named strategy is absent locally says so and offers the paste
  field, rather than falling back to a default player.
- The checkbox player keeps rendering as prose exactly as it does today; the
  by-reference form appears only when a rule set is in use. Nothing regresses for
  the user who never opens the Rules section.
- This gives the deferred **saved-scenario library** item in [v-next.md](v-next.md)
  its concrete shape: the library is a directory of hashed strategies, and the
  shared-seed enforcement that item demands is unchanged and still required.

Hashing is FNV-1a over the canonical compiled byte stream, matching
[`SimConfig::fingerprint`](../crates/craps-app/src/config.rs) — hand-rolled,
platform-stable, no serde.

## 11. Explorer and Duel

Custom strategies join the Explorer as additional rows with a user-supplied
label, crossed with progressions and quit rules exactly as the eleven curated
ones are. The common-random-numbers discipline is untouched: every combination
still plays the same dice under `session_seed`, so head-to-head rankings stay
paired and the tie machinery keeps working.

**Guardrail:** the curated eleven remain the default sweep. Custom strategies are
opt-in per row, because 11 → *n* strategies is a combinatorial multiplier on a
sweep whose cost is already the honest limit on session count. The Explorer
never silently trades *n* for breadth; if adding a strategy would drop sessions
below the CI the leaderboard needs, it says so.

## 12. Explicit Non-Goals

- **No Turing completeness, no embedded scripting language.** Rhai, Lua, or a
  WASM sandbox would be less work up front and would cost the Scenario Sentence,
  the static checks, the cost bound, and the performance budget — and would put
  an unreviewed general-purpose program on every core. The bounded language gives
  up almost nothing real; §7 is the evidence.
- **No I/O, clock, RNG, or ambient state inside a strategy.** Principle 6 is the
  foundation of every paired comparison in the app.
- **No mid-roll or call bets.** The decision point is between rolls, because that
  is where a player's decisions are.
- **No strategy marketplace, sharing service, or import-from-URL.** A local
  directory of text files, and the clipboard.
- **No new bet types in this release.** The language is designed so buy, lay,
  hop, and horn are one enum variant each; adding them is its own milestone with
  its own edge derivations in `edge.rs`, not a rider on this one.
- **No "optimize my strategy" search.** The Explorer ranks what the user asks it
  to rank. An optimizer over a user-authored parameter space is a different
  product and invites exactly the overfitting the app exists to argue against.
- **No advice.** Unchanged from GUI spec §12: the app never names a best
  strategy, and the language does not change that.

---

# Part II — Implementation Plan

## 1. Target Architecture

New module tree in the engine; the app grows one editor and one panel.

```
crates/craps-engine/src/strategy/
    mod.rs        Strategy trait, Builtin(BetSelection), the decide() contract
    action.rs     Action, BetRef, Amount, adjudication + RejectReason
    view.rs       TableView, FeatureMask, derived-history accumulators
    ast.rs        Rule, Trigger, Condition, Expr — the one AST
    compile.rs    AST -> Program (slot resolution, group expansion, checks)
    program.rs    Program, Op, StratState — the executed form
    text.rs       parse() / render() — the text serialization + round-trip law
    library.rs    the twelve progression fragments as rule fragments

crates/craps-app/src/
    rules/editor.rs   the rule table (typed slots, drag reorder, fire counts)
    rules/prose.rs    Span/FragmentId rendering for a rule tree
    rules/bench.rs    the Bench panel
    rules/store.rs    the local strategy library + hashing
```

## 2. Engine Surface Changes

All changes are flagged. The existing verification suite moves verbatim and must
stay green at every milestone; `pinned_outcomes.rs` is the gate on all of it.

**S1 — the intent surface.** `Action`, `BetRef`, `Amount`, and one adjudicator
`fn apply(&mut Session, Action) -> Result<(), RejectReason>` holding every
money-moving line that today lives inline in `place_bets`. `place_bets` becomes
the built-in strategy *expressed in actions* — same decisions, same order, same
cents. Gate: bit-identical outcomes over the pinned vectors and the equivalence
battery. No user-visible change ships in this step.

**S2 — `TableView` and feature masks.** The read-only state borrow, plus derived
accumulators (`hits`, `streak`, `wins`/`losses` per stream) maintained behind a
`FeatureMask` the compiler emits. Default player: mask empty, zero added work,
verified by the throughput gate.

**S3 — `BetEventKind::Rejected { reason }`.** ~10 new emit sites inside the
adjudicator, all under the existing `RollObserver` discipline, so `Noop` still
monomorphizes to nothing.

**S4 — the compiled program.** `Program { ops: Vec<Op>, trigger_masks: Vec<u32>,
slots: u16, features: FeatureMask, hash: u64 }`, built once per run and shared
immutably across the rayon pool. Per-session state is
`StratState { vars: [i64; SLOTS] }` — fixed size, stack-allocated, no
allocation and no hashing in the loop. Trigger dispatch is a bitmask test
against the roll's event set, so a roll that matches no rule costs one `and`.

**S5 — the progression library.** The twelve `Progression` variants re-expressed
as rule fragments, attachable per stream. `Progression`'s labels and
`description()` strings survive verbatim; `progression_preview` continues to
drive the Design screen sparklines, computed by running the fragment. Gate: the
`progression_transition_sequences` test passes unmodified against the compiled
fragments.

**S6 — sweep and explore.** `ExploreRow` carries a strategy id that may point at
a custom program; `SweepConfig` carries the `Arc<Program>`. `session_seed` and
the CRN contract are untouched — this is additive.

## 3. The Performance Budget (binding)

The engine's whole premise is hundreds of millions of rolls per second. A
strategy interpreter in the inner loop is the one thing in this document that
could break the app.

**Budget:** a compiled strategy costs **≤ 2× the built-in path** on the
pass-line throughput benchmark, and a compiled *reproduction* of the built-in
player costs **≤ 1.15×**. Both are `#[ignore]`d release-mode gates in the same
file as the existing `throughput` test, run in CI's perf tier.

Means: no allocation per roll; no hashmaps (names resolved to slot indices at
compile time); `i64` register machine with a small fixed operand stack; trigger
bitmasks so most rolls short-circuit; feature masks so unread history is never
computed; `Program` shared by `Arc`, never cloned per session.

**Decided:** the hand-written `place_bets` fast path stays until the compiled
reproduction is inside 1.15×, then it is **deleted**. Two policies that must
agree is a permanent tax on trust; the point of S1 is to end with one place
where money moves, not two.

## 4. Milestone Roadmap

Every milestone ends green, buildable, releasable. The checkbox player works
identically at every one of them.

**P0 — Intent surface (6.0 dd).** S1 + S3. `place_bets` rewritten as actions;
rejections emitted; Replay's ledger shows them. Exit: pinned outcomes bitwise
identical, equivalence battery green, no user-visible change.

**P1 — State view (3.0 dd).** S2, with the feature-mask machinery and the
zero-cost-when-unused throughput gate. Exit: default-player throughput within
noise of P0.

**P2 — AST, compiler, interpreter (9.0 dd).** S4 + S5. The eleven curated
strategies and twelve progressions ported to rule sets. Exit: every ported
strategy reproduces its hand-written twin bitwise over 10k seeds; both
performance gates green.

**P3 — Text form (5.0 dd).** `parse`/`render`, the `language 1` header, the
round-trip property test over randomized rule sets, error messages that name the
offending token (the pattern
[sentence.rs](../crates/craps-app/src/sentence.rs) already establishes). Exit:
strategies are authorable by paste; the feature is usable by a power user with
no GUI work at all.

**P4 — The Bench (6.0 dd).** Rule attribution events, the stepper panel, the
adjudication ledger, per-rule fire counts. Built *before* the editor
deliberately: §8 is what makes the editor's output trustworthy, and it is the
debugging surface for P2 and P3 themselves.

**P5 — The rule editor (10.0 dd).** Typed-slot rows, prose rendering,
drag-reorder, the static checks of §9 wired into the existing order-ticket
validation strip. The expensive item, and the cut line (risk 2).

**P6 — Library and provenance (4.0 dd).** The strategy directory, content
hashing, sentence-by-reference, STALE-on-hash-mismatch, missing-strategy state.

**P7 — Explorer and Duel (4.0 dd).** S6, custom rows, the opt-in guardrail and
its caption, per-stream progressions in the Anchor.

**P8 — Hardening and release (5.0 dd).** Accessibility of the editor (keyboard
path through every slot, AccessKit summaries per rule), fuzzing the parser,
malformed-library states, CHANGELOG, v0.5.0.

**Total: 52 developer-days.** P0–P3 (23 dd) is the load-bearing half and carries
zero user-visible risk; everything after is interface.

## 5. Testing & CI

- **Equivalence (non-negotiable).** Every ported strategy and progression
  reproduces its hand-written twin bitwise over 10k seeds. This is the entire
  safety net for P0–P2 and it is cheap, because `pinned_outcomes.rs` already
  exists.
- **Round-trip law.** `parse(render(ast)) == ast` and `render(parse(text)) ==
  text` over randomized rule sets, mirroring the sentence codec's existing law.
- **Adjudication tests.** One test per `RejectReason`, asserting the event fires
  and the layout is unchanged.
- **Static-check tests.** One authored strategy per §9 diagnostic, asserting the
  exact sentence shown.
- **Parser fuzzing.** Malformed text never panics and always names a token.
- **Performance gates.** The two ratios in §3, release-mode, perf tier.
- **Interaction tests.** kittest over the editor: add rule, reorder, dead-rule
  badge, Bench step, STALE on hash mismatch.

## 6. Dependency Budget

**Zero new dependencies.** The parser is hand-rolled recursive descent over the
tokenizer pattern `sentence.rs` already uses; the hash is the existing FNV-1a;
the library is `std::fs` over plain text. `serde` remains rejected for the same
reason it is rejected today — the text form *is* the serialization, and a
derived one would be a second, divergent grammar.

## 7. Risk Register

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| 1 | Interpreter breaks the throughput premise | Med | High | Two binding ratios in §3 gated in CI from P2; feature masks; bitmask dispatch; built-in path retained until the gate passes, then deleted |
| 2 | 52 dd overruns — this is a language project inside a simulator | High | High | Cut line: P5 (the editor) drops to v0.5.1. P0–P4 ship a complete, usable feature — authored as text, debugged in the Bench — and the checkbox Design screen is untouched for everyone else |
| 3 | S1 refactor perturbs resolution order | Med | High | Pinned outcomes + equivalence battery gate P0; no behavior change is permitted in the same commit as the refactor |
| 4 | Users author strategies that silently do nothing | High | Med | Principle 4 end-to-end: rejection events, Bench fire counts, dead-rule and never-bets static checks, Run disabled with a sentence |
| 5 | Grammar churn breaks saved strategies | Med | Med | `language N` header is mandatory; unknown versions are refused, never guessed; grammar changes ship a migration or a new version, never a silent reinterpretation |
| 6 | Explorer combinatorics eat session count | Med | Med | Curated eleven stay the default; custom rows opt-in; the sweep refuses to trade *n* below the CI the leaderboard needs and says so |
| 7 | Sentence contract weakened by by-reference strategies | Med | Med | Hash mismatch is STALE in the existing amber register; missing strategy is an explicit state; the checkbox player's sentence is byte-identical to today's |
| 8 | The language grows without limit under feature requests | High | Low | §12 is the contract; new vocabulary must be justified by a strategy that cannot otherwise be written, and enters through `TableView`, not through new syntax |

## 8. Deferred

- **New bet types** (buy, lay, place-to-lose, hop, horn). One `BetKind` variant
  each plus resolution and a closed-form edge; own milestone.
- **Record-my-play authoring** — play a session by hand in the Bench and
  generalize the actions into rules. High value, and it needs the editor to
  exist first.
- **Strategy diffing** — two rule sets side by side with their behavioral
  divergence on shared dice. The Duel already answers the outcome question; this
  would answer the *why*.
- **Cross-strategy optimization.** Permanently out, per §12.
