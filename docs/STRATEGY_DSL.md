# The Player

## Craps-Sim Strategy Language — Design Specification & Implementation Plan

**Status:** In progress against v0.5. P0–P4 have landed; §3, §5.2, §6 and
Part II §3 have been corrected against what building them taught, with the
original claim left visible above each correction.

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

press half-press for place 6          # per stream; bare `press x` sets all

on point-established:
    bet place inside 2 units          # $22 inside at a $5 table

on roll when point != 0:
    bet pass                          # no amount: whatever this stream presses to

on win of place 6:
    set hits = hits + 1
    press place 6 to stake(place 6) * 2

on roll when hits >= 4:
    down all place

on seven-out:
    set hits = 0

on roll when profit <= -$200 or profit >= $150:
    leave "enough"
```

`parse(render(ast)) == ast` and `render(parse(text)) == text`, both
property-tested over 400 randomized rule sets — the same law the Scenario
Sentence already obeys. Every example in §7 is written this way and simulated,
because a text form that could not say them would be a subset with a nicer
face rather than the second editor Principle 2 claims.

Three things the sketch above got wrong before the grammar existed, corrected
here rather than quietly:

- **A condition belongs to a rule, not to a statement.** The original sketch
  nested `when hits == 2: regress …` inside a body. There is no such form: a
  rule is `on <trigger> [when <expr>]:` and its body is unconditional. Nesting
  would have made the tree and the text different shapes, which is the one
  thing Principle 2 forbids.
- **Per-stream pressing says `for`, not `on`.** `press martingale on dont pass`
  and `press martingale` followed by `on seven-out:` cannot be told apart
  without the parser knowing every trigger word — a grammar that needs that
  lookahead breaks the day a trigger is added.
- **Operators need spaces.** `dont-pass` and `hits-this-shooter` are single
  words, so `a - b` is subtraction and `a-b` is an identifier that does not
  exist. `-200` is still a negative literal. This is what lets every bet be
  spelled the way craps spells it without a symbol table in the tokenizer.

**An amount is optional on `bet`**, and leaving it off means whatever that
stream's pressing calls for — the base stake under a flat progression. `bet
pass` is how a player says it; making them write `bet pass pressed` at a
table where nothing presses would be the language describing the engine
rather than the game.

Groups (`place inside`, `all hardways`) are parse-time sugar: they expand into
one statement per member and never render back as a group, because the tree
holds the members and the law is about the tree.

Expressions carry the brackets they need and no others. The first renderer
parenthesized everything, on the grounds that the tree is the truth and the
text is only its serialization — and then the Bench put those rules on screen
and `on roll when ((point != 0) and up(pass)):` turned out to be nothing
anyone would write. Precedence belongs to the parser already; the renderer
only has to agree with it, which the round-trip law checks.

The `language 1` header is binding: the parser refuses a version it does not
know rather than guessing, so a saved strategy never silently changes meaning
under a grammar revision.

## 6. Progressions Are Declared Per Stream

*Revised at P2b, against this section's original claim.* What follows is what
was written first, and then what building it taught.

**The original claim** was that Martingale is not a feature but a pair of
rules — `on loss of pass: press pass to 2x` — and that the twelve progressions
would therefore dissolve into rule fragments.

**What building it showed** is that this models the game wrong. When a place
bet hits, the dealer pays you and asks *right then* whether to press it, before
the next roll leaves the shooter's hand. The engine has always done exactly
that: pressing happens inside resolution, out of the winnings. A rule fires at
the decision point, which is *after* every bet on that roll has resolved — so
compiling a progression to rules moves the press to a different moment, changes
which bet gets the last dollar when the bankroll is short, and silently stops
reproducing the engine it was supposed to replace.

The decision point is the right place for *"take everything down after two
hits"*. It is the wrong place for *"press this winner out of its own
winnings"*, and the difference is not a technicality — it is where the money
moves.

**So progressions stay declarative, and become per stream.** A strategy
declares `progressions: [Progression; 17]`, one per betting stream, applied
where the bet resolves. Rules keep the decision point. A bet asks for
[`Amount::Pressed`] — whatever its stream's system currently calls for — which
under a flat progression is exactly the base stake, which is why a flat player
never has to say which it meant. Two things follow, both of which the original
claim also wanted:

1. The Explorer's progression axis (GUI spec §6.6) is preserved exactly — the
   twelve are named, enumerable, and crossable as they are today.
2. Per-stream progressions become expressible for the first time: *Martingale
   the don't pass, flat place bets.* Today `Progression` is one global enum and
   this cannot be said. This also retires the "per-bet Anchor variants beyond
   progressions" item in [v-next.md](v-next.md) by giving it a real
   representation.

The gate that proves it: all eleven curated strategies crossed with all twelve
progressions, simulated both ways on the same dice, agreeing on the roll they
died, what they walked out with, their peak outlay, and their handle
(`compiled_matches_builtin_across_every_progression`). Mixed pressing is proven
distinct from both all-flat and all-Martingale, because a test that could not
tell them apart would not be testing anything.

The existing [`Progression`](../crates/craps-engine/src/bets.rs) enum and its
`description()` strings survive as the library's labels; the GUI picker and its
sparklines are unchanged.

## 7. Worked Examples

Each is a strategy that could not be expressed before the language
existed. **This section is generated** from
`crates/craps-engine/src/strategy/examples.rs`, where every one of them
is parsed, compiled and simulated by the test suite. It was hand-written
once and had drifted out of agreement with the grammar within a
milestone — naming triggers that do not exist and an `in` operator that
was never built — so it is no longer hand-written.

They ship in the app under **Examples**, as demonstrations of the
language and not as advice: most are bad bets and one is deliberately
superstitious.

**Press twice, then collect**

> *Place the 6 and 8; press each on its first two hits; regress after
> that.*

The thing that could not be said before: a bet whose size depends on how
many times its own number has come.

```
strategy "Press twice, then collect" language 1

on come-out:
    bet pass base

on roll when point != 0 and point != 6:
    bet place 6 base

on roll when point != 0 and point != 8:
    bet place 8 base

on win of place 6 when hits-this-shooter(6) <= 2:
    press place 6 to stake(place 6) * 2

on win of place 6 when hits-this-shooter(6) > 2:
    regress place 6 to base

on win of place 8 when hits-this-shooter(8) <= 2:
    press place 8 to stake(place 8) * 2

on win of place 8 when hits-this-shooter(8) > 2:
    regress place 8 to base
```

**Off until the shooter proves himself**

> *Place bets off after a seven-out until the shooter makes a point.*

Memory, and a bet that sits on the felt resolving nothing.

```
strategy "Off until the shooter proves himself" language 1

var trusted = 0

on seven-out:
    set trusted = 0

on point-made:
    set trusted = 1

on roll when point != 0:
    bet place 6 base
    bet place 8 base

on roll when trusted == 0:
    working place 6 off
    working place 8 off

on roll when trusted == 1:
    working place 6 on
    working place 8 on
```

**Stop loss and stop win**

> *Stop at −$200 or +$150, whichever comes first.*

```
strategy "Stop loss and stop win" language 1

on come-out:
    bet pass base

on roll when profit >= $150 or profit <= -$200:
    leave "enough"
```

**The field is due (superstition)**

> *Bet the field only after two field numbers in a row.*

Nonsense, faithfully modeled. Principle 5: a language that could only
express sound play could not refute unsound play, and refutation is the
product.

```
strategy "The field is due" language 1

var streak = 0

on roll when last-total <= 4 or last-total >= 9:
    set streak = streak + 1

on roll when not (last-total <= 4 or last-total >= 9):
    set streak = 0

on roll when streak >= 2:
    bet field base
```

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

**Measured, and the budget corrected.** The figures below are
nanoseconds per simulated roll, single-threaded, compiled strategy against
hand-written player on identical dice (`bench_compiled`):

| configuration | rules | built-in | compiled | ratio |
|---|---|---|---|---|
| pass line | 1 | 22.5 ns | 29.4 ns | 1.31× |
| 3-point molly | 9 | 35.8 ns | 77.2 ns | 2.16× |
| loaded table | 29 | 55.9 ns | 214.9 ns | 3.85× |
| loaded + full press | 29 | 62.1 ns | 225.8 ns | 3.64× |

Cost scales with rule count at roughly 5 ns per rule per roll, which is what
a dispatching interpreter costs and is not going to become free. The 1.15×
figure this section previously carried was written before anything was
measured; it was wrong, and it was wrong in a way that mattered, because it
implied a plan the numbers do not support.

P2c raised the loaded figures from 182 to 215 ns — an 18% tax on interpreted
strategies, paid for the actions that make the language worth having. The
built-in player pays none of it.

**Budget, revised:** ≤ 2× for a strategy of up to ten rules, and ≤ 4.5× for
one that covers the felt. The gate asserts the worst case across the four
benchmark configurations as a regression tripwire, not as a target to
optimize toward — and it is set with headroom on purpose, because a tripwire
four percent above the current reading is a tripwire that fires on noise.

This is fast enough. A loaded custom strategy at 182 ns/roll is ~5.5M
rolls/s/core — a 1.2M-session run still finishes in seconds on any machine
that runs this app.

Means, all of which are in: no allocation per roll (the proposal buffer and
strategy memory live on the session, not on each decision's stack); no
hashmaps, names resolved to slot indices at compile time; a stack machine
over `i64` with a fixed operand depth; guards fused into the rule header for
the shapes real rules actually use, which was worth 1.7× on its own; an
already-working check in the interpreter so a bet that is up never walks to
the table; layout sums computed only when a strategy reads them; a 16-byte
instruction with a size tripwire; `Program` shared by `Arc`, never cloned.

**Decided, reversing this document's earlier position:** the hand-written
`place_bets` fast path **stays permanently**. The original plan was to delete
it once the compiled reproduction came within 1.15%, on the grounds that two
policies that must agree is a tax on trust. The measurement says that
threshold is unreachable, so the choice is between a permanently slower
checkbox player and two implementations — and the thing that made the second
option frightening no longer applies: `from_selection` compiles the checkbox
player into the language, and the equivalence test simulates both against
each other over 10,000 seeds per curated strategy, asserting the roll they
died on, what they walked out with, their peak outlay, and their handle. Two
implementations pinned to each other by an executable proof are safer than
one implementation with no independent check. The tax on trust is paid by the
test, not by the reader.

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
strategy reproduces its hand-written twin bitwise over 10k seeds; the
performance gate green.

- **P2a — done.** AST, compiler with fused guards and static checks, stack
  machine, `Player` seam on the session loop, `from_selection`, and the
  10,000-seed equivalence proof across all eleven curated strategies.
- **P2b — done.** Per-stream progressions (S5), `Amount::Pressed`, and the
  equivalence proof extended across the full progression axis. The design
  revision in §6 was found here.
- **P2c — done.** `Press`, `Regress`, `Down`, `Working`, and `Leave` on the
  intent surface, with §7's worked examples built and simulated as tests.
  Contract bets refuse to come down; odds always come down; a bet called off
  neither wins nor loses. One gap left deliberately: `Working` says off or
  on, and cannot yet say *working on the come-out* — that needs the come-out
  branch of `resolve` to grow place-bet resolution it has never had, and it
  is its own change with its own risk to the pinned outcomes.

**Interaction, decided:** a progression sets the stake where the bet
resolves; a rule may then override it at the decision point. Last write
wins, the same ordering that governs two rules touching one bet.

**P3 — Text form (5.0 dd). Done.** `parse`/`render`, the `language 1` header,
the round-trip property test over 400 randomized rule sets, and error messages
that name the offending token and its line (the pattern
[sentence.rs](../crates/craps-app/src/sentence.rs) already establishes).
Strategies are authorable by paste, and a parsed strategy is proven to play
the identical session to the tree it came from — the round-trip law extended
through the compiler and the engine to the money. The §5.2 corrections were
found here.

**P4 — The Bench (6.0 dd).** Rule attribution events, the stepper panel, the
adjudication ledger, per-rule fire counts. Built *before* the editor
deliberately: §8 is what makes the editor's output trustworthy, and it is the
debugging surface for P2 and P3 themselves.

- **P4a — done.** The engine half: `bench_session` returns a per-roll ledger
  of which rules fired, what each asked for, and what the table did about it,
  plus fire counts covering every rule so a dead one reads as zero rather
  than as absent. Watching is proven not to change what is watched.
  `BetEventKind::Won` gained `stake_returned`, because a ledger that cannot
  tell a pass-line win from a place-bet win cannot account for the rail, and
  §8 promises exactly that accounting.
- **P4b — done, then rebuilt after a design review.** The first version was
  a collapsed panel at the bottom of the Design screen with a transport of
  its own. The review's verdict — *"a debugger you cannot debug with"* — was
  right: it reimplemented a worse transport than the one Replay already had,
  bound no keys, showed no dice strip, and left `BenchTrace::refusals()`
  computed and uncalled, so a refusal on roll 91 of 137 could only be found
  by clicking ninety-one times.

  It now splits along the seam the information architecture already had.
  **Authoring** is on Design behind a `Checkboxes | Rules` control, because
  Design builds the player and choosing to write a strategy is the same act
  as choosing to play one — there is no second switch to disagree with that
  one. **The ledger** is on Replay, which owns the transport, the keyboard,
  the dice strip and the population envelope; §8 always said the Bench
  should reuse them. A strategy night is benched rather than merely traced,
  so `as_session_trace` lets every existing view read it without knowing.

  Also from the review, and all of it correct: the run conditions are stated
  (the table minimum decides refusals and went unsaid); the session's
  refusals are listed together, each one a way to reach it; the night's
  ending is told; instructions-per-decision and a debug-printed feature mask
  came off a Saturday user's screen; and the colours came back inside
  Principle 2 — a rule that fired is raised onto the next surface rather
  than painted blue, a rule that never fired is demoted rather than alarmed,
  and its `0×` carries the meaning without depending on colour at all.

**P4c — The run path (1.5 dd). Done.** A design review found the hole this
roadmap left: no milestone before P7 connected a compiled `Program` to a
simulation run, so a strategy could be authored, compiled and stepped, and
then Space would silently run the checkbox player instead. `SweepConfig`
carries an optional program; one place decides which player is live; the
rail says so and the exported provenance says so; a strategy selected but
not compiled refuses the run rather than falling through. Risk 2's cut line
("P0–P4 ship a complete, usable feature") was false until this landed.

**P6a — Persistence (2.0 dd). Done.** Pulled forward from P6 for the same
reason: nothing persisted an authored strategy, so every one written was
destroyed on quit. A directory of plain-text `.craps` files beside
`prefs.txt` — the text form is already the serialization, so there is
nothing here for serde to do — with save, open, and a delete that asks
first. An editor holding work that is not on disk says so.

What this deliberately leaves for P6 proper is §10's by-reference form: the
Scenario Sentence does not yet carry a strategy's name and hash. That
touches the sentence codec and its round-trip law, and belongs with the
provenance story rather than riding along with a file store.

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
| 1 | Interpreter breaks the throughput premise | Med | High | **Retired at P2a.** Measured at 1.24x–3.39x by rule count (§3); the built-in path is retained permanently rather than deleted, and the two are pinned to each other by a 10,000-seed equivalence test |
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
