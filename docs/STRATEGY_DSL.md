# The Player

## Craps-Sim Strategy Language — Design Specification & Implementation Plan

**Status:** P0–P8 have landed. A review of the branch
([STRATEGY_DSL_REVIEW.md](STRATEGY_DSL_REVIEW.md)) then found that parts of
this document described an API, an evaluation model and a test suite the code
never had — so §3, §4, §5.2, §7, §9 and Part II §3 and §5 have been corrected
against what building them taught, with the original claim left visible above
each correction. The gaps are enumerated in that review; what is written here
is what the code does.

Some of those gaps have since been closed by building the thing rather than by
rewording the claim: the table's own numbers and `working(bet)` as reads
(§3.2), the `everything` group (§3.3), `press … by` (§3.3), lists walked in
step (§4), all six static checks (§9), and a version gate that refuses forward
and reads back (§5.2). Those passages are additions rather than corrections,
and they say so where the distinction matters. What remains undone is in
Part II §8, which is the honest list.

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
| Dice & point | `point` (0 = off), `come-out`, `last-total`, `roll`, `rolls-this-shooter`, `shooter` |
| Money | `cash`, `wealth`, `profit`, `peak-profit`, `drawdown`, `handle`, `buy-in` |
| Table | `table-min`, `table-max` |
| Layout | `up(bet)`, `stake(bet)`, `working(bet)`, `live-come`, `live-dont-come`, `come-point(n)`, `dont-come-point(n)`, `on-table-face` |
| History | `hits(n)`, `hits-this-shooter(n)`, `wins(bet)`, `losses(bet)`, `streak(bet)`, `paid(bet)` |
| Memory | user-declared variables and flags |

*Corrected against the shipped `Read`, which this table was wrong about in
both directions.*

**`live-come` counts both halves of a come bet's life.** It was listed here
for two revisions with no definition anywhere, and the ambiguity is real
enough that two people building the same three-point strategy guarded against
both readings: a come flat sitting in the box has not reached a number yet,
and it was never said whether it counted. It does. `live-come` is the flat in
the come box, if there is one, plus every come point established — so the
three-point Molly's cap is `live-come < 3`, and a flat still travelling
occupies one of those three exactly as a player waiting on it would say it
does. `live-dont-come` counts the same way on the dark side.

**It listed `working(bet)`, and for two revisions there was no such read.** A
strategy could turn a place bet or a hardway off and back on (§3.3) and could
not then ask whether one was on — the one place in the language where
something could be written and not read, recorded here rather than left to be
found by whoever needed it.

*That asymmetry is closed.* `working(bet)` is a `Read` and a method on the
view, which is exactly the shape the paragraph above predicted the fix would
take: a variant and a field, not a grammar change. It answers off-or-on for
the two kinds of bet that can be called off, and for everything else it
answers what `up(bet)` answers — a bet that cannot be turned off is working
whenever it is up, and giving the same question two answers depending on which
bet was asked about would be the language describing its own implementation.
The standing workaround — remember it in a `var` set beside the `working`
statement that did it — was advice for a gap that no longer exists, and a
strategy written that way still works but no longer has to be.

**It stopped short of what the parser accepts.** `come-out`, `come-point(n)`
and `dont-come-point(n)` exist and are listed above. `come-point(n)` is how a
rule asks between rolls what `come point on n` (§4) announces at the moment it
happens — a come flat that is sitting on a box number, rather than one that
just arrived there.

**And three reads are new, because nothing could say them.** `buy-in` is what
the player sat down with. `table-min` and `table-max` are the table's own two
numbers. These matter more than a vocabulary line usually does, because
without them a whole class of strategy is only accidentally correct: a
stop-loss written `profit <= -$200`, or a Martingale that stops climbing at a
figure typed in dollars, is a sentence about *one* table. It is true at the
minimum it was written at and quietly wrong at every other — and the table
minimum is an axis the Explorer sweeps, so the same strategy is scored at four
of them in a single run. Seven of the twelve strategies in the expressiveness
experiment hard-coded a number that should have been one of these three, and
nothing said so, because a wrong constant is not a syntax error. Written as
`profit <= -buy-in / 2`, or `stake(pass) * 2 > table-max`, the strategy travels
to the table it is played at instead of the one it was typed at.

All values are `i64` cents or counts. **There is no floating-point arithmetic in
the language** — money is integer cents end to end, as everywhere else in this
engine, and a strategy that could compute `$12.333…` would break payout
exactness.

Derived history (`hits`, `streak`, …) is maintained only when a compiled
strategy actually reads it. The compiler emits a feature mask; the session
maintains exactly those accumulators and nothing else, so the default player
pays nothing for vocabulary it never uses (S2).

### 3.3 What a strategy may do — `Action`

*Corrected against the shipped surface.* **The original claim**, left visible
here because half of it was never built:

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
// Amount = Cents | Units(n) | Multiple(n, of) | Max | Half | All
```

**What shipped:**

```rust
enum Action {
    Bet(BetRef, Amount),          // put it up if it is not up; idempotent
    SetStake(BetRef, Amount),     // move a working bet to a new stake
    Down(BetRef),                 // take it down where the table allows
    Working(BetRef, bool),        // off/on — place bets and hardways only
    Leave,
}
```

Five differences, each of which is a decision rather than an omission:

- **Odds are a bet, not a verb.** There is no `Odds` action, because there did
  not need to be one: `BetRef::PassOdds`, `ComeOdds(n)`, `DontPassLay` and
  `DontComeLay(n)` are bets like any other, and `bet` puts them up. In text
  that is `bet odds on pass max`. **This is the single thing most likely to
  stop somebody learning the language** — five independent implementers
  produced the same `expected an action — found "odds"` before inferring the
  spelling from nothing, in a game whose only zero-edge bet is odds. The four
  spellings are `odds on pass`, `odds on dont pass`, `odds on come <n>` and
  `odds on dont come <n>`, and they read anywhere a bet reads.
- **`press` and `regress` take `to`, never `by`.** Both compile to
  `SetStake`, which is a target and not a delta; the verb is what says which
  direction the target must lie in, and a `press` whose target is at or below
  the current stake is refused rather than quietly applied (§3.4). "By X" is
  said as `to stake(place 6) + X`, which the expression grammar already
  reaches.

  *Corrected: `by` is now spelled too.* `press place 6 by $6` and `regress
  place 8 by 1 unit` parse, and the bullet above is the reason they had to.
  "Press it" is the sentence a player actually says, and it is a *step* — the
  dealer is asked for six dollars more, not for a destination of twenty-four.
  Making the author compute the destination by hand, in cents, with the payout
  unit worked out first, was the language asking them to do the table's
  arithmetic. So `by` is parse-time sugar: it becomes `stake(<that bet>) +
  <step>`, the same destination the long form writes, with `by n units`
  multiplied out through `table-min` on the way. It does not survive
  rendering. The tree holds the destination, one press is one shape, and the
  rule editor never has to learn a second — the same convention as bet groups
  (§5.2), and for the same reason: the round-trip law is about the tree.
- **`set` is a statement, not an action.** Memory never goes to the table, so
  it never enters the proposal buffer; it is written inside the decision, at
  the moment the statement runs, which is the semantics §4 now states.
- **`Leave` carries nothing.** `leave "enough"` parses and the reason is
  discarded — it is a note for whoever reads the strategy, not a value the
  engine holds, and it does not render back.
- **`Amount` is smaller.** It is `Base | Pressed | Cents(i64) | Units(i64) |
  MaxOdds`. `Base` is the table's own stake for that bet — the minimum rounded
  up to its payout unit, or the configured prop stake. `Pressed` is whatever
  the bet's stream currently calls for (§6), which under a flat progression is
  exactly `Base`. `Multiple(n, of)`, `Half` and `All` were never built, and
  nothing in the twelve-strategy experiment asked for them. In the tree an
  amount is an *expression* — `AmountExpr::Cents(Expr)` and
  `AmountExpr::Units(Expr)` — so `to stake(place 6) * 2` and `min(cash / 4,
  $50)` are amounts, which is a better feature than the three that were
  dropped.

In text, then, the whole surface is:

```
bet pass                          # no amount: whatever the stream presses to
bet pass base                     # the table's own stake for this bet
bet place 6 $18                   # money is written as money
bet place 6 1800                  # a bare number is cents — the same bet
bet place inside 2 units          # n table minimums, per member
bet odds on pass max              # the most the odds policy allows
press place 6 to $24
press place 6 by $6                # a step, not a destination
bet place 5 1 unit                 # the singular parses too
regress place 6 to base
down all place
down everything                    # the place numbers and the hardways
working place 6 off               # and `working place 6 on`
leave                             # `leave "enough"` reads the same to the engine
set hits = hits + 1
```

`BetRef` names a bet or a **group**: `place inside` (5, 6, 8, 9), `place
outside` (4, 10), `all place`, and `all hardways` (`all hard` reads the same).
Groups are sugar, expanded at parse time into one statement per member.

*Corrected: `everything` exists, and it is not everything.* This section
previously recorded the group as promised-but-never-built, on the grounds that
a bet on every stream at once is a sentence nobody wanted to write. That
reasoning was right and the conclusion was wrong — the group people wanted was
never "every bet". `everything` expands to the ten bets that can be taken down
or called off: the six place numbers and the four hardways. Not the line,
which is a contract once the point is on; not the come bets, which travel and
are contracts too; not the one-roll propositions, which resolve before the
question could be asked. So `down everything` is the sweep a player makes when
they are leaving, and it sweeps exactly what a dealer would sweep — a group
whose membership is the answer to a rule of craps rather than a list somebody
chose. Written any other way it would half-fail on every use, refusing four
bets (§3.4) to take down six.

Adding a bet type the engine does not yet model (buy, lay, place-to-lose, hop,
horn) is one `BetKind` variant plus its resolution arm. It is not a grammar
change. The language is designed so the layout can grow under it.

### 3.4 Adjudication

The table applies an `ActionList` in order and emits an event per action. New,
in [trace.rs](../crates/craps-engine/src/trace.rs):

```rust
BetEventKind::Rejected { reason: RejectReason, what: Attempted }
```

*Corrected against the shipped enum.* **The original claim** was
`RejectReason ∈ { BelowTableMinimum, AboveTableMaximum, NotAllowedNow,
InsufficientBankroll, NoSuchBet, AlreadyDown, ClippedToMax }`. **What ships is
ten reasons, and they are not those seven:**

| Reason | What the table said |
|---|---|
| `LineBetWithPointOn` | the point is already established |
| `NeedsPointOn` | there's no point yet |
| `NumberIsThePoint` | that number is the point |
| `ContractBet` | a contract bet can't come down |
| `NothingThere` | there's nothing on that bet |
| `NoOddsAllowed` | odds policy allows none |
| `BelowTableMinimum` | below the table minimum for that bet |
| `WrongDirection` | a press can't lower, a regress can't raise |
| `InsufficientBankroll` | bankroll won't cover it |
| `NotAllowedNow` | none of the above — odds with no flat behind them, calling off a bet that cannot be called off |

The differences worth stating:

- **`ClippedToMax` is not a refusal.** It is a `BetEventKind` of its own,
  emitted where the table maximum truncates a named stake, and the bet still
  goes up at the maximum. Putting it among the refusals confused an outcome
  with a reason for having none. The design point is unchanged and is now
  enforced rather than described: the truncation that stops a Martingale is an
  event the user sees, not a flat spot in a curve to be inferred.
- **`AboveTableMaximum` therefore does not exist** — a stake above the maximum
  is clipped and said so, not refused.
- **`NotAllowedNow` was split.** One label covering three different rules of
  craps — you cannot make a line bet with the point on, you cannot make a
  come, place or hardway bet without one, you cannot place the point number —
  told an author nothing about which rule they broke, and five implementers
  mapped the legality matrix (§3.5) by probing it. The three now have names,
  and `NotAllowedNow` is left for the residue.
- **`NoSuchBet` and `AlreadyDown` were never needed.** A bet that is not on the
  felt refuses as `NothingThere`; a bet that is already up is a no-op, because
  `bet` is idempotent by design and asking twice is not an error. A bet that
  does not exist at all — `place 7` — is refused by the parser and again by the
  compiler, before any table sees it.
- **A refusal now says what was refused, and for how much.** The event carries
  an `Attempted` — betting, pressing, regressing, taking down, turning off or
  on — beside its reason, and the event's `stake_cents` is the stake that was
  asked for rather than the zero that moved. Without the verb, a ledger could
  only report that *something* about a bet was refused: six rules can touch
  one place number, and "place 6 — there's nothing on that bet" does not say
  whether a press, a take-down or a working toggle went looking for it.
  Without the amount, a zero-stake bet from a counter that had not been
  initialized read as *bankroll won't cover it* beside a full bankroll, which
  is a true sentence that sends the reader to the wrong page.

Per Principle 4, every rejection surfaces: each of the ten is emitted as a
`Rejected` event and reads in the Bench in the words of the table above,
with the verb and the stake the rule asked for.

### 3.5 What the table will take, and when

Which bets the table will take at which moment is table rules rather than
language rules, and this document stated it nowhere — so it was learned by
refusal-probing, which is a poor way to read a specification. Verified against
the adjudicator:

| Bet | When it may be made |
|---|---|
| `pass`, `dont pass` | on the come-out only — once the point is on they are refused |
| `come`, `dont come` | only with a point on |
| `hard 4/6/8/10` | only with a point on |
| `place n` | only with a point on, and never on the number that *is* the point |
| `odds on …` | only with the flat it backs already up, and only where the odds policy allows something behind that point |
| `field`, `any seven`, `any craps` | any time; they resolve on the next roll whatever it is |

And what may be moved once it is out there: **only place bets and hardways can
be turned off and on** — line and come bets are contract bets, odds ride with
what they back, and a one-roll bet resolves before the question could be
asked. `pass` and `come` cannot come down once the point is on (`ContractBet`);
odds always come down, which is most of why they are the best bet on the table.

One of these diverges from a real casino and does so knowingly: a real table
will usually sell you the point number as a place bet, and this one refuses it
unconditionally. Five of the twelve strategies in the expressiveness
experiment wanted it, and what they played instead diverged in money from what
they were written to play. That is a modeling choice to close or to argue for,
not a rule of craps, and it is named here so a strategy that wanted it knows
why it was refused. It is now also carried as a named deferral in Part II §8,
because a divergence that lives only in a matrix row reads as settled.

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

### The table is frozen; memory is not

*Corrected against the interpreter, and this is the correction that matters
most in this document.* The paragraph above is half the truth, and it reads as
the whole of it: one snapshot, every rule against it, actions afterward. Eight
of twelve independent implementers had to find the other half by experiment
before their counters were safe, and two came away from the same page with
opposite models. What one decision actually does, in three parts:

- **Table reads are a snapshot.** `point`, `cash`, `stake(place 6)`, `hits(6)`
  — every read in §3.2 — come from one view built before the first rule is
  considered and never rebuilt. A rule cannot see a bet an earlier rule asked
  for in the same breath, because nothing has reached the table yet.
- **Actions are buffered.** Every `bet`, `press`, `regress`, `down`, `working`
  and `leave` goes into a proposal buffer and is applied after every rule has
  been read, in the order the rules were written. That order is what decides
  which bet gets the last dollar when the bankroll is short.
- **`set` is immediate.** A `set` writes its slot the moment the statement
  runs, and every rule below it in the same decision reads the new value.
  Memory is neither buffered nor part of the snapshot.

So rule order is load-bearing control flow for anything stateful — every
counter, every stage machine, every rung of a progression written as rules
rests on it:

```
var hits = 0

on win of place 6:
    set hits = hits + 1

on win of place 6 when hits >= 3:
    down place 6
```

The second rule sees the count the first one just wrote, so the bet comes down
on the third hit. Swap the two rules and it comes down on the fourth, because
the guard would read the count as it stood before this win. Both orders parse,
both compile, both run, and the Bench shows both rules firing either way — the
difference is only in the money. §7's *Off until the shooter proves himself* is
the same mechanism used deliberately: its `set trusted = 0` on `seven-out` is
read by the `on roll` rules beneath it in that same decision.

The asymmetry is deliberate rather than accidental. A rule is a description of
what the player wants, and a condition that silently depended on an earlier
rule's *bet* would be unreadable at a table where the bet may be refused;
memory is the strategy talking to itself, and buffering that would make a
counter unable to count. But it was undocumented, which is the defect: a
semantic every stateful strategy depends on was discoverable only by
experiment.

**Triggers.** `session-start`, `come-out`, `point-established`, `point-made`,
`seven-out`, `roll`, `total(n)`, `come point on n`, `dont come point on n`,
`win of <bet>`, `loss of <bet>`.

`come point on n` is a come flat reaching a box number, which is a different
event from the table's own point being established and had no name until an
ergonomics assessment went looking for one. Without it the only way to act on
an establishment was to remember the previous state and compare — four rules
per number, and easy to get subtly wrong.

**Conditions.** Any boolean expression over §3.2 reads: comparisons, `and`,
`or`, `not`, `+ - * / min max`, integer only.

**Memory.** `var name = <number>` declares a session-scoped counter, reset to
that number at the start of every session. Fixed count, resolved to slot
indices at compile time; there is no dynamic namespace.

*Corrected twice.* This said `var name = <expr>`. The initializer is a literal
number — `0`, `4`, or money written as money, `$150` — and never an
expression, because a slot's starting value is decided before there is a table
to read. And that number is now **honoured**: for two milestones it was parsed
and thrown away, every slot started at zero whatever the page said, and a
strategy quietly played a different system than the one written. Three
implementers lost an attempt to it, and it is the kind of silent wrongness
Principle 4 exists to forbid.

A name the grammar already spends is refused rather than shadowed. `point`,
`cash`, `profit`, `roll`, `min`, `max` and the rest of the bare-word reads,
operators, amount words and keywords are reserved, and `var point` is an error
that says so. The parameterized reads are *not* reserved — `var hits`, `var
streak`, `var paid`, `var stake`, `var up`, `var wins`, `var losses` all
declare fine, because those words only read as reads when a `(` follows them.
Declaring the same name twice is refused too: two slots with one name is not
something to resolve by a rule.

**Bounded iteration.** `for each of 4, 5, 6, 8, 9, 10 as n { … }` — over a
list written out in full, and nothing else (Principle 3). The block is read
once per value with `n` bound to it, and `n` may stand anywhere a number may:
`place n`, `hard n`, `odds on come n`, `come-point(n)`, `hits-this-shooter(n)`,
`total(n)`, or a bare term in an expression. Blocks nest, and an inner binding
shadows an outer one.

A binding is not memory. It is a number the parser substitutes while reading
the block, so what comes out is exactly the rules somebody would otherwise
have typed six times. A name already used for a memory slot is refused rather
than shadowed, because the two are different things and sharing a name would
hide that.

**Several lists may be walked in step.** `for each of 6, 8 as n with 8, 6 as
other { … }` reads the block twice, with `n` and `other` bound together — 6
and 8, then 8 and 6. This is what a pair of numbers needs in order to say
something about each other, and it is the commonest thing in the game: the 6
and the 8 are partners in half the strategies anybody writes, and *"when this
one wins, take the other one down"* had no way to be said. A single-list block
can bind `n` but not `n`'s partner, so that sentence was written out per
number — four lines becoming twelve rules, each with a box number typed by
hand and each a chance to type the wrong one. Any number of lists may be
joined with `with`; the first one's length decides how many iterations there
are.

Two things it refuses. **Lists walked together must be the same length** — a
short list would have to be either an error or a silent stop partway through
the long one, and the second is the kind of quiet wrongness Principle 4 exists
to forbid. **A name may not be bound twice in one block**, because `with 8, 6
as n` beside `of 6, 8 as n` is a sentence with no reading: one of the two
bindings would have to win, and neither is the obvious winner.

**The block survives, and stops surviving honestly.** A strategy records the
block beside the rules it produced — its name, its values, and its own text
verbatim, comments and spacing included. Whether it is *still* a block is
never remembered: it is asked, by re-reading that text once per value and
comparing to the rules sitting there. So the three cases fall out with no
flag that can disagree with reality:

- folded, and edited as a block — the edit reaches every iteration, they
  still agree, it stays a block;
- unfolded and looked at — nothing changed, so it is still a block;
- unfolded and edited apart — two iterations now differ, so they are not one
  rule, and it stops being drawn or written as one.

A record that has stopped being true is pruned before rendering, which is why
`parse(render(s)) == s` remains a law rather than failing on a claim that had
already expired. Rewriting the block is the way back, which is honest:
at that point it genuinely is not one rule any more.

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
- **Operators need spaces.** A hyphen belongs to the word it sits in:
  `hits-this-shooter`, `last-total`, `peak-profit`, `point-established`,
  `seven-out` and `half-press` are each one token. So `a - b` is subtraction
  and `a-b` is an identifier that does not exist. `-200` is still a negative
  literal. This is what lets every bet and trigger be spelled the way craps
  spells it without a symbol table in the tokenizer.

  *Corrected:* this bullet gave `dont-pass` as one of its single words. The
  parser wants two — `dont pass`, `dont come`, and `odds on dont come 6` — and
  the hyphen is a parse error. `hits-this-shooter` is right. The rule the
  bullet exists for is unchanged; only half its evidence was.

**Money is written as money.** `$150` and `$12.50` are read to the cent, and
`$1,200.05` may carry the separator. A **bare number in an amount is cents**,
which is the sharpest edge in the grammar and points at the most natural
sentence somebody can write: `bet place 6 12` is a twelve-*cent* bet. The table
now refuses it as below its minimum (§3.4) rather than rounding it up into a
six-dollar bet nobody asked for, so the trap costs a refusal with a reason
instead of three hundred rolls of wrong money. The renderer writes money back
as money — a positive constant amount, and a bare number compared against a
money read, both come out as `$150` rather than as `15000`, so opening a
hand-written strategy in the rule editor and saving it no longer rewrites it
into cent-soup. A negative literal stays in plain cents; `-$200` is a negation
applied to a positive literal, comes back written that way, and is a different
tree from `-20000`, which is what the round-trip law is about.

**An amount is optional on `bet`**, and leaving it off means whatever that
stream's pressing calls for — the base stake under a flat progression. `bet
pass` is how a player says it; making them write `bet pass pressed` at a
table where nothing presses would be the language describing the engine
rather than the game.

**`2 units` and `1 unit` are both how a person writes it**, and the singular
now parses. It used to fall through to the cents rule and then choke on the
word — an error message about a token, on the most natural way there is to
write the smallest bet the table takes.

Groups (`place inside`, `all hardways`, `everything`) are parse-time sugar:
they expand into one statement per member and never render back as a group,
because the tree holds the members and the law is about the tree. `press <bet>
by <amount>` (§3.3) is sugar under the same rule and renders the same way —
back out as the `to` destination it desugared to, because that is what the tree
holds. Both are cases of one principle: the text form may offer two ways to
say a thing, and the tree may hold only one, or the round-trip law and the
rule editor would each have two shapes to reconcile.

Expressions carry the brackets they need and no others. The first renderer
parenthesized everything, on the grounds that the tree is the truth and the
text is only its serialization — and then the Bench put those rules on screen
and `on roll when ((point != 0) and up(pass)):` turned out to be nothing
anyone would write. Precedence belongs to the parser already; the renderer
only has to agree with it, which the round-trip law checks.

**The `language 1` header is binding, and it refuses in one direction only.**
A version *newer* than this engine knows is refused: a file written against a
grammar with words in it that are not here would be misread rather than
rejected, and misreading is the one failure a save format may not have. A
version *older* is read.

The first gate did both, and doing both was wrong. Every grammar change so far
has been additive — a trigger, a read, an amount, a group — and an additive
change leaves an old file meaning exactly what it meant when it was written.
Turning those away would have faced the next release with a choice between
refusing every file on every user's disk and never bumping the number at all,
which is how a version gate becomes decorative. So the policy is stated rather
than implied: **an additive change leaves the number alone and old files keep
reading; a breaking change bumps it, and old files are then migrated or
refused deliberately.** That is the whole answer to risk 5 in the register —
grammar churn breaking saved strategies — and it is an answer rather than a
hope, because it is the only reading under which a saved strategy either runs
as it was written or says why it will not.

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

for each of 6, 8 as n {
    on roll when point != 0 and point != n:
        bet place n base
}

for each of 6, 8 as n {
    on win of place n when hits-this-shooter(n) <= 2:
        press place n to stake(place n) * 2

    on win of place n when hits-this-shooter(n) > 2:
        regress place n to base
}
```

*And for two milestones it did not climb.* A progression re-prices its bet
where the bet resolves, and the stream still had this one at its base — so
every press was torn back down by the very win it was riding, and the second
press recomputed the first one forever. The example was the counterexample to
its own caption, and nothing said so: the money looked plausible and the rules
fired. A `press` or `regress` at the decision point now tells that bet's
stream what the bet is worth from this moment, which is what lets any ladder
climb at all; §6's interaction is unchanged, and the example is pinned by a
test that walks away once the 6 stands at $24.

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
spec §6.1's order-ticket validation.

**The original claim was six checks**, and a previous revision of this section
recorded that one shipped, one was computed and shown nowhere, and four did
not exist. **All six exist now**, in
[check.rs](../crates/craps-engine/src/strategy/check.rs). What follows is what
they do, and — as important — what they do not.

### One refusal, five sentences

The design turns on a distinction that is easy to lose and expensive to lose:
**never-bets refuses to compile; nothing else stops a run.** A strategy that
can never put money at risk has nothing to simulate, so it is a
`CompileError`, Run is refused, and the sentence is *"This strategy never
places a bet, so there is nothing to simulate."* Every other check is a
`Diagnostic` — a thing worth saying about a strategy that still runs.

That is not leniency, it is Principle 5. A dead rule is legal. A hedge that
cannot hedge is legal. The app exists to model a belief exactly and draw what
happens, and a compiler that refused unsound play could not refute unsound
play — refutation is the product. So the checks below never gate anything.
They exist so that nobody is surprised by their own rules.

### Two entry points, because two of them need a table

`check(strategy, program)` answers from the strategy alone: **dead rule**,
**conflict**, **cost**. `against_table(strategy, rules, table_min, budget)`
answers the two that cannot be asked without knowing where the strategy is
being played: **exposure**, which is measured against a budget, and
**clipping**, which is measured against a maximum. The compiler does not know
either — it compiles a strategy once and the same program is then run at every
table minimum the sweep covers — so asking it would have meant either
compiling per table or answering with a number true of only one of them.
Whoever holds the configuration asks the second function, with the minimum and
the budget in hand.

- **Never bets.** No reachable rule places a bet. The only one that refuses.
- **Cost.** `Program::cost_bound` — instructions walked per decision in the
  worst case, every rule firing and every guard passing, across *n* rules. It
  is the number Principle 3 buys, and it is now shown rather than merely
  computable. It is not a warning and is not drawn as one (below).
- **Dead rule.** A rule whose condition can never hold: *"Rule 3 can never
  fire: its condition is never true"*, or *"…its conditions cannot all hold at
  once."*
- **Conflict.** Two rules acting on the same bet at the same trigger, naming
  which one wins.
- **Exposure.** The most the strategy could have on the layout at once,
  against the configured budget.
- **Clipping.** A pressing system that will meet the table maximum, and on
  which step.

### What the dead-rule check does not do

**It is deliberately not a theorem prover, and saying so is the point.** It
folds the arithmetic a person actually writes — constants, negation, `not`,
and the short-circuiting halves of `and` and `or`, so `when 0 and <anything>`
is dead however complicated the other half is — and then looks for the two
contradictions people actually make: asking one read to be two numbers (`point
== 4 and point == 6`), and asking for a window with nothing in it (`profit >
$100 and profit < $50`). Comparisons written either way round are the same
claim, because a person writes both.

Everything subtler is out of reach and stays out of reach. A rule dead because
of what an earlier rule wrote to memory, a rule shadowed on every path by one
above it, a condition unsatisfiable only given the rules of craps — none of
these is caught, and the Bench's per-rule `0×` after a run is what answers for
them, as it always did. That is a real limit and it is written here rather
than left to be discovered, because the alternative is the overclaim this
document has already been caught making once: a section that promises
completeness teaches an author to read a clean strip as a proof, and a clean
strip is not a proof.

The other half of not crying wolf is the tests: alongside every condition the
check must catch is a set of ordinary conditions it must leave alone —
`point != 0 and point != 6`, `hits(6) >= 2 and hits(8) >= 2`, an `or` over a
profit window. A diagnostic on any of those would teach authors to ignore the
strip, which costs more than the check earns.

### Why conflict only fires on unconditional rules

Two rules touching one bet at one trigger conflict **only when neither carries
a condition**. Two rules that each have a `when` may perfectly well be
describing different moments — that is the most ordinary way there is to write
a strategy, and *"press it on the first two hits, regress after that"* in §7 is
exactly that shape. Diagnosing it would cry wolf on nearly every real
strategy. Where both are unconditional there is no such reading: they fire
together, always, and one of them is simply overwritten. The sentence says
which: *"Rules 1 and 2 both act on place 6 at the same trigger; rule 2 is
written later, so it wins."* `working` and `leave` are not stakes and conflict
with nothing.

### Exposure is a floor when it cannot be a figure

Worst-case outlay counts one stake per distinct bet, because a bet is one slot
however many rules mention it, and takes the largest stake any rule names for
it. Where an amount is *written* — `$60`, `2 units` — that is exact. Where it
is *computed* — `stake(place 6) * 2`, `min(cash / 4, $50)`, or anything the
table answers like `base`, `pressed` and `max` — there is no static answer, so
the bet contributes its base and the whole figure is reported as a floor:
*"This strategy can have **at least** $120 on the layout at once, and the
budget is $50."* A floor that says it is a floor is worth having; a number
that quietly under-reports because one amount was multiplied is not.

### Where they surface

In the Design screen's order-ticket strip, beneath the rule count and the
cheapest bet, in the register that strip already uses for the bet rail —
because a rule set and a checkbox set are two ways to build the same player,
and their validation should not read like two different features. Warnings are
amber, which is the register the app reserves for something that is wrong: a
dead rule, a conflict, a clip, and an exposure that exceeds the budget. An
exposure *within* budget is a reassurance and the cost line is an engine
disclosure, so neither takes amber; they are plain text, one line each, said
because the reader is entitled to them and not because anything needs doing.

What the compiler *does* refuse, beyond never-bets, is its own limits rather
than these diagnostics: a memory slot used but never declared, more slots or
rules or per-decision actions than the fixed state holds, an expression nested
deeper than the operand stack, a bet the table does not have, and a win/loss
trigger on odds — which have no record of their own, because they resolve with
the flat behind them. Each renders as a sentence in the same register.

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
| pass line | 1 | 22.09 ns | 29.66 ns | 1.34× |
| 3-point molly | 9 | 34.97 ns | 76.64 ns | 2.19× |
| loaded table | 27 | 54.49 ns | 199.16 ns | 3.65× |
| loaded + full press | 27 | 60.88 ns | 208.64 ns | 3.43× |

Cost scales with rule count at roughly 5 ns per rule per roll, which is what
a dispatching interpreter costs and is not going to become free. The 1.15×
figure this section previously carried was written before anything was
measured; it was wrong, and it was wrong in a way that mattered, because it
implied a plan the numbers do not support.

P2c raised the loaded figures from 182 to 215 ns — an 18% tax on interpreted
strategies, paid for the actions that make the language worth having. The
built-in player pays none of it. They then came back down to 199, and not by
anyone optimizing for it: a bare `point != 0` — the commonest single condition
in the language, and four rules of the loaded configuration — had no arm in
the guard fuser and compiled to general stack code beside a fast-path variant
that sat unreachable. It now fuses into one test, like every other guard shape
real rules take, and the loaded ratio fell from 3.85× to 3.65×.

**Budget, corrected again.** This section carried *"≤ 2× for a strategy of up
to ten rules, and ≤ 4.5× for one that covers the felt."* The second half held.
The first was never true of the numbers printed directly above it — the
3-point molly is nine rules and has measured 2.16×–2.19× since it was first
benchmarked. A budget its own table contradicts is worse than no budget,
because it reads as a gate somebody is holding.

**The budget is ≤ 2.5× at ten rules or fewer, and ≤ 4.5× across the felt.**
Both halves are asserted by `bench_compiled` — the small-strategy half was
never asserted at all until this revision — and both now run in CI (§5). They
are regression tripwires across the four benchmark configurations rather than
targets to optimize toward, set with headroom on purpose, because a tripwire
four percent above the current reading is a tripwire that fires on noise.

This is fast enough. A loaded custom strategy at ~200 ns/roll is ~5M
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

**P6 — Library and provenance (4.0 dd). Done.** The Scenario Sentence
carries a strategy the way §10 designed it: `playing "44 Inside, regressed"
#9f3c1a2b at 3-4-5× odds`, replacing the bet-rail fragment rather than
joining it, because describing bets that are not in play beside the strategy
that is would be the sentence contradicting itself. A pasted sentence
resolves the reference against the local library and says *found*,
*changed*, or *missing* — never falling back to the bet rail, which would
attribute a scenario to a player that never played it.

Three things this forced, each of which was a real defect:

- **The odds policy was gated on a checkbox that is not in play.**
  `SimConfig::rules` derived it from `sel.take_odds`, so a strategy asking
  for `max` odds would have been refused at every table where the bet rail
  happened to have odds switched off.
- **The bet rail was part of a strategy's scenario when it should not be.**
  Editing it struck strategy results stale for no reason. `canonical()` now
  states what the scenario actually is, and the round-trip law is stated
  over that rather than over every field of the struct.
- **The sentence lower-cased strategy names.** Names are user-chosen and are
  also file names; the normalizer now folds everything except what is inside
  quotes, which is the one part of a sentence whose spelling this parser
  does not own.

The pinned configuration fingerprint changed, deliberately and for the first
time: which player is live is part of what a run was cut from.

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

**P5 — The rule editor (10.0 dd). Done.** Typed-slot rows on Design's Rules
tab: a trigger from a closed list, conditions as `<read> <comparison>
<value>` clauses joined by "and", and actions as verb-bet-amount. Money
reads ask for dollars and counts ask for counts. Nothing here writes or
parses text — the rows edit the tree, the tree renders to text, and the
round-trip law is what keeps the two editors from becoming two languages.
Rules reorder and delete; each carries its fire count from the last night
stepped in Replay, which is where a dead rule announces itself. §9's checks
join the order-ticket strip that already answers the same question of the
bet rail — all six of them, per §9 as rewritten: never-bets refuses the run,
and the other five say their sentence beside it without stopping anything.

**Principle 2, narrowed against what building it taught.** The claim was
that anything the text expresses, the rows can render *and edit*. Rendering
holds: every rule shows. Editing does not, and should not. A condition
outside the clause shapes — an `or`, nested arithmetic — is displayed as its
own text with the row saying to edit it there, and a computed amount like
`stake(place 6) * 2` the same. Slots for arbitrary expression trees would be
a worse editor than the line of prose they replaced, and building one would
be the interface describing the AST rather than the game. The half of the
principle that is load-bearing — that neither editor can say something the
other cannot hold — is unchanged and tested.

**P6 — Library and provenance (4.0 dd).** The strategy directory, content
hashing, sentence-by-reference, STALE-on-hash-mismatch, missing-strategy state.

**P7 — Explorer and Duel (4.0 dd). Done, and narrowed.** The Duel runs an
authored strategy against any curated combination on identical dice, which is
the honest paired comparison and the valuable half of this milestone; a
strategy dueled against the selection it was compiled from is proven identical
on every session, which is the pairing checking itself. The Explorer enters the
strategy as its own rows beside the curated eleven — crossed with the quit
rules only, because its pressing is in its rules and a progression axis would
ask it to be something it already answers for itself.

**Narrowed against the original plan:** custom strategies crossed with every
progression, and the combinatorial guardrail that would then be needed, are
deferred past v0.5. Four extra combinations per minimum against 528 does not
touch the session count, which is what that guardrail existed to protect; a
full cross would, and nothing yet asks for one. A strategy has no closed form,
so an authored side reports no edge rather than an invented one and the Duel's
verdict talks about shape where it cannot talk about cost.

**P8 — Hardening and release (5.0 dd). Done.** Every rule row exposes its own
prose as its accessible name, because a row is a dozen widgets and a screen
reader landing on it should hear the rule rather than twelve dropdown labels.
A file in the library can be anything — hand-edited, half-saved, written
against a grammar this build does not know — so opening a broken one says what
is wrong with it and leaves the text in the editor to be fixed. CHANGELOG and
v0.5.0.

**Parser fuzzing, replaced with something that runs.** A fuzzing harness for a
local single-user app's strategy parser is machinery nobody would run again.
What matters is that a person mistyping a strategy gets a sentence rather than
a crashed window, so the test asserts exactly that over every truncation and
every single-byte deletion of a valid strategy, plus the malformed shapes worth
naming: nothing panics, and every refusal names its line.

**Total: 52 developer-days.** P0–P3 (23 dd) is the load-bearing half and carries
zero user-visible risk; everything after is interface.

## 5. Testing & CI

*Corrected against what actually runs.* This section described a suite in the
present tense, and two of its entries were the proofs the whole architecture
leans on — sitting `#[ignore]` and absent from CI, which meant they were
enforced by nobody.

- **Equivalence (non-negotiable).** Every ported strategy and progression
  reproduces its hand-written twin bitwise over 10k seeds. This is the entire
  safety net for P0–P2 and it is cheap, because `pinned_outcomes.rs` already
  exists. It is `#[ignore]` because it is a minute of work, not because it is
  optional — and it **now runs in CI**, in a dedicated `proofs` job in
  [ci.yml](../.github/workflows/ci.yml), which exists precisely so the ordinary
  test matrix stays fast without the deep proof going unrun. The default tier
  still runs the 400-seed and 150-seed versions on every platform.
- **Performance gates.** Both ratios in §3, release-mode. Also `#[ignore]`, and
  also run by the `proofs` job, which invokes it with `--ignored` rather than
  leaving it to a perf tier nobody has. Until this revision it was unrun *and*
  asserted only the 4.5× half; both halves are asserted now.
- **Round-trip law.** `parse(render(ast)) == ast` and `render(parse(text)) ==
  text` over randomized rule sets, mirroring the sentence codec's existing law.
  The generator reaches every bet reference, trigger, operator, read, statement
  and progression the grammar spells, plus non-zero memory initializers and
  hostile strategy names. What it still does not generate is a `for each`
  block, which is the one shape whose round trip is asked rather than
  remembered (§4) — so that shape is covered by named tests rather than by the
  law.
- **Adjudication tests.** One test per `RejectReason`, asserting the event fires
  and the layout is unchanged.
- **Static-check tests.** One authored strategy per §9 diagnostic, asserting the
  exact sentence shown. §9's checks all existing now, that is what
  [check.rs](../crates/craps-engine/src/strategy/check.rs) tests: a dead rule
  in each of its two shapes, a conflict naming the later rule, the cost line,
  exposure inside and outside a budget, a computed amount turning the exposure
  into a floor, and a Martingale meeting a 4× maximum. Two of these are
  negative and matter as much as the rest: ordinary conditions that must not
  be called dead, and a conditional pair that must not be called a conflict —
  a check that cries wolf teaches the author to stop reading the strip. The
  compiler's own refusals, the limits it enforces on slots, rules, actions and
  expression depth, are still not each pinned to their sentence, and should be.
- **Parser fuzzing.** Malformed text never panics and always names a token.
  What runs is the truncation-and-deletion sweep described in P8 rather than a
  fuzzer: every truncation and every single-byte deletion of a valid strategy,
  plus the malformed shapes worth naming.
- **Interaction tests. Deferred.** kittest over the editor — add rule, reorder,
  dead-rule badge, Bench step, STALE on hash mismatch — does not exist. What
  the headless suite does cover on the strategy path is the seam P4c and P6
  built: that a selected strategy becomes the live player or the run is
  refused, that editing one strikes its results stale, that the exported
  provenance names the player that played, and that the bet rail is not part of
  a strategy's scenario. The rows themselves are untested. The dead-rule check
  is now built and tested in the engine (above); what is untested is its
  *appearance* — that the sentence reaches the order-ticket strip and takes
  amber there — which is the same kittest gap as everything else in this
  bullet rather than a missing check.

## 6. Dependency Budget

**Zero new dependencies.** The parser is hand-rolled recursive descent over the
tokenizer pattern `sentence.rs` already uses; the hash is the existing FNV-1a;
the library is `std::fs` over plain text. `serde` remains rejected for the same
reason it is rejected today — the text form *is* the serialization, and a
derived one would be a second, divergent grammar.

## 7. Risk Register

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| 1 | Interpreter breaks the throughput premise | Med | High | **Retired at P2a.** Measured at 1.34x–3.65x by rule count (§3); the built-in path is retained permanently rather than deleted, and the two are pinned to each other by a 10,000-seed equivalence test that now runs in CI |
| 2 | 52 dd overruns — this is a language project inside a simulator | High | High | Cut line: P5 (the editor) drops to v0.5.1. P0–P4 ship a complete, usable feature — authored as text, debugged in the Bench — and the checkbox Design screen is untouched for everyone else |
| 3 | S1 refactor perturbs resolution order | Med | High | Pinned outcomes + equivalence battery gate P0; no behavior change is permitted in the same commit as the refactor |
| 4 | Users author strategies that silently do nothing | High | Med | **Retired.** Principle 4 end-to-end: rejection events carrying the verb and the stake asked for, Bench fire counts, never-bets refusing the run with a sentence — and now the dead-rule and conflict checks (§9), so an unsatisfiable condition and an overwritten rule are both named in the strip before a run rather than inferred from a `0×` after one. What the checks cannot reach is stated in §9 rather than papered over; the Bench's fire counts remain the backstop for it |
| 5 | Grammar churn breaks saved strategies | Med | Med | `language N` header is mandatory and refuses in one direction: newer than this engine knows is refused rather than misread, older is read, because every change so far has been additive and additive changes leave old files meaning what they meant (§5.2). A breaking change bumps the number, and old files are then migrated or refused deliberately — never silently reinterpreted |
| 6 | Explorer combinatorics eat session count | Med | Med | Curated eleven stay the default; custom rows opt-in; the sweep refuses to trade *n* below the CI the leaderboard needs and says so |
| 7 | Sentence contract weakened by by-reference strategies | Med | Med | Hash mismatch is STALE in the existing amber register; missing strategy is an explicit state; the checkbox player's sentence is byte-identical to today's |
| 8 | The language grows without limit under feature requests | High | Low | §12 is the contract; new vocabulary must be justified by a strategy that cannot otherwise be written, and enters through `TableView`, not through new syntax |

## 8. Deferred

- **Working on the come-out.** `working <bet> off` and `on` ship; *working on
  the come-out* does not. The come-out branch of `resolve` turns place bets
  and hardways off unconditionally and has never had a resolution arm for
  them, so there is nothing for the flag to switch. This is engine work with
  its own risk to the pinned outcomes, not a grammar change, and it is the one
  gap P2c left deliberately.
- **Placing the number that is the point.** A real table will usually sell it
  to you; this one refuses it unconditionally, as `NumberIsThePoint` in
  `flat_spec` (§3.5). Five of the twelve strategies in the expressiveness
  experiment wanted it, and what they played instead diverged in money from
  what they were written to play — so this is a named divergence from a
  casino rather than a rule of craps, and it belongs on this list rather than
  in the legality matrix as though it were settled. Closing it is a resolution
  question (a place bet on the point number and the line bet behind it both
  resolving on the same total) before it is a language one.
- **New bet types** (buy, lay, place-to-lose, hop, horn). One `BetKind` variant
  each plus resolution and a closed-form edge; own milestone.
- **Record-my-play authoring** — play a session by hand in the Bench and
  generalize the actions into rules. High value, and it needs the editor to
  exist first.
- **Strategy diffing** — two rule sets side by side with their behavioral
  divergence on shared dice. The Duel already answers the outcome question; this
  would answer the *why*.
- **Cross-strategy optimization.** Permanently out, per §12.
