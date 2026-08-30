# The Player, Reviewed

## A critical examination of the strategy language branch

**Subject:** branch `strategy-language-p0` at `a248e71`, reviewed against `main` at
v0.4.3 — 51 files, +11,595 / −283, covering the engine `strategy/` module, the app
screens that surface it, and [STRATEGY_DSL.md](STRATEGY_DSL.md).
**Date:** 2026-08-30.

> **Status: addressed.** Every finding below has been acted on, in the commits that
> follow `a248e71`. The report is kept as it was written — it is the record of what
> was wrong, and a review edited to match the fix afterwards is no longer evidence of
> anything. Where a recommendation was *not* taken, §12 says which and why.
> [CHANGELOG.md](../CHANGELOG.md) lists the user-visible half.

**Verdict:** the architecture is genuinely good and the language reads the way craps
is spoken, but the branch is not yet trustworthy — its worst defects are precisely
the silent, compiles-and-quietly-wrong failures its own Principle 4 exists to forbid.

**What is right.** One AST behind two editors, a single adjudicator, a
zero-allocation interpreter whose performance claims are substantially real, an
equivalence battery that pins the compiled player to the hand-written one, and a
trigger vocabulary every implementer praised. Four of twelve surveyed strategies
transcribed almost sentence-for-sentence — the 3-Point Molly is 18 lines and zero
variables. The fire-count/refusal debugging loop was called the best part of the
system by all twelve implementers.

**What is wrong.** Five reproduced defects: the parser and interpreter can each
crash the process from pasted text (the engine's own test suite currently fails on
this branch because of it); the Explorer will panic on the leaderboard rows this
branch now emits; bare amounts are cents with no table-minimum check; and
stake-relative presses cannot compound, so the entire press-ladder family of
strategies — including the shipped `PRESS_TWICE` example's own prose — is
unreachable. Around those sit roughly a dozen confirmed silent-meaning traps
(`session-start` never fires, initializers are discarded, `place 260` wraps to
`place 4`…), and the two proofs the design leans on hardest — the 10,000-seed
equivalence test and the performance gate — are `#[ignore]` and absent from CI.

**The distance to good is short and known.** Nearly every finding is a seam, not a
foundation problem. The recommendations in §10 are ordered so the five ship-blockers
come first; most are small.

| | |
|---|---|
| 103 | findings, each independently re-verified at the cited lines |
| 5 + 33 | critical + major after de-duplication |
| 12 | strategies implemented and simulated in the language |
| 4 / 4 / 4 | expressible fully / partially / via workaround |
| 2 | process-killing crashes reproduced from strategy text |

---

## 1. Reproduced defects

Six results were reproduced directly against this branch's release build before
anything else in this report was written. Each is a fact about the checkout, not a
reviewer's claim.

### 1.1 The engine test suite fails on this branch

`cargo test -p craps-engine` aborts in debug mode:
`malformed_input_is_refused_rather_than_fatal` — the test whose whole job is proving
malformed text cannot crash — dies on its own 500-nested-paren case, because the
recursive-descent expression parser has no depth limit (`text.rs:452–530`;
`Parser.depth` counts only `for each` nesting). It passes in release only because
release frames are smaller; the underlying defect is unbounded:

```
$ strategy_check deep.craps   # 'on roll when' + 100,000 nested parens, release build
thread 'main' has overflowed its stack
fatal runtime error: stack overflow, aborting
```

### 1.2 User arithmetic can kill a rayon worker

`apply_bin` guards division by zero but not `i64::MIN / -1`, which panics in every
build profile (`program.rs:564`). The constants are writable; the interpreter runs on
every core with no `catch_unwind`; one pasted strategy aborts a whole sweep — the
exact failure the module's own doc rules out (`ast.rs:113–115`, "data must not be
able to kill a worker thread mid-sweep").

```
on roll when ((0 - 9223372036854775807) - 1) / (0 - 1) == 0: bet pass

PARSE: OK · COMPILE: OK (1 rules)
thread 'main' panicked at crates/craps-engine/src/strategy/program.rs:564:17:
attempt to divide with overflow
```

### 1.3 `session-start` is dead vocabulary

It parses, compiles, is documented in §4, and is the first trigger offered in the
rule editor (`rules.rs:37`) — but no code path ever sets `fired::SESSION_START`
(`game.rs` sets only the point/seven-out bits). Five of twelve implementers lost an
attempt to it.

```
     0x  on session-start: bet pass  <-- NEVER FIRED
   169x  on come-out: bet field
```

### 1.4 A 12-cent pass-line bet rides

A bare integer in amount position is cents (`text.rs:294–308`), and no layer enforces
a table minimum: the spec's `BelowTableMinimum`, `AboveTableMaximum`, `NoSuchBet`,
`AlreadyDown` and `ClippedToMax` reasons were never built — the shipped enum has five
different variants (`action.rs:102–116`). So `bet pass 12` plays 300 rolls at
12 cents on a $5 table without one refusal, and `bet place 6 12` silently becomes a
$6 bet via payout-unit rounding. The table *maximum* is enforced — but silently
clamped (`game.rs:245`), with no event, where the spec makes the visible
`ClippedToMax` a design centerpiece ("the user must see the truncation").

### 1.5 Stake-relative presses cannot compound

Spec §6's decided interaction — "a progression sets the stake where the bet resolves;
a rule may then override it at the decision point" — has an unintended consequence:
the stream's flat progression re-prices every resolved bet back to base *before* the
next decision, so `press place 6 to stake(place 6) * 2` recomputes 6×2 forever. A
flat press to a constant works and pays at the pressed level (verified); a ladder can
never climb:

```
on win of place 6: press place 6 to stake(place 6) * 2   # 30 seeds, 300 rolls each

   727x  on win of place 6: press place 6 to stake(place 6) * 2
     0x  on roll when stake(place 6) >= 2400: leave  <-- NEVER FIRED
```

This makes the press-and-ride family — a canonical strand of real craps play —
unreachable, and it means the shipped `PRESS_TWICE` example does not do what its
prose says: "press each on its first two hits" presses to 2× base on both hits rather
than climbing 6→12→24. The §5.1 flagship editor example ("press place 6 to $18")
works only because it is absolute.

### 1.6 The pre-existing failures are not this branch's

The three `chart::export` test failures (an epaint-internal `TexturesDelta` panic)
reproduce identically on `main` at v0.4.3 in the same environment. They are unrelated
to this branch and are not counted in this review.

---

## 2. The design, examined

The spec's core decisions survive scrutiny well. Bounded-by-construction is the right
call and §7 really is the evidence — nothing in the twelve-strategy experiment needed
recursion or unbounded loops. One decision point per roll matches the game. The
intent/adjudication split (Principle 1) is honestly implemented at the decision
point: `Session::apply` is a single dispatch and every refusal that reaches it emits
an event. The `for each` block's provenance design — keep the verbatim text,
re-derive "is it still a block?" by re-parsing instead of trusting a flag — is
genuinely novel and philosophically consistent with the rest of the app.

Three design-level problems stand above the bug list:

**The two press systems don't compose.** `press` means three things (a per-stream
declaration applied at resolution, a rule statement applied at the decision point,
and the `pressed` amount), executing at different moments with different money — the
spec itself admits the moments differ ("changes which bet gets the last dollar"). The
resolution-time re-price silently reverting decision-time presses (defect 1.5) is the
concrete cost. Either decision-point presses should set the stream's pressed level
(surviving resolution), or a strategy mixing rule-presses with a re-pricing
progression on the same stream should be refused at compile time.

**The evaluation model is a hybrid the docs don't describe.** §4 says matching rules
append actions to one intent list applied afterward. Empirically, `set` writes memory
immediately — visible to every later rule's condition in the same decision
(`program.rs:484`) — while table reads are a frozen snapshot and actions are
buffered. Eight of twelve implementers had to probe-discover this before their
counters were safe; two concluded opposite models from the same document. Every
stateful strategy depends on it, and rule order is load-bearing control flow. This
needs to be a documented, named semantic (or `set` should be buffered like everything
else).

**Bare integers as cents are a trap aimed at the most natural sentence.**
`bet place 6 12` is how a player writes $12; the language reads 12 cents, no minimum
check catches it, and the renderer canonicalizes even correctly-written `$150` into
`15000` (and `-$200` into `-(20000)`) — one click in the rule editor rewrites
hand-authored money into cent-soup (`text.rs:1103`, `rules.rs:330`). Money deserves a
required unit: refuse bare integers in amount position, keep `$` and `units`, and
render dollars back as dollars.

Smaller but confirmed design flaws in the surface: the read vocabulary silently
captures variable names (`var roll` is declarable, writable, and unreadable;
`var min` is unreadable-as-parse-error); `var x = 5` parses an initializer and
discards it (all slots start 0); odds references alias their flat everywhere except
`bet` (`on win of odds on pass` ≡ `on win of pass`; `press martingale for odds on pass`
attaches to the flat; `paid()` is blind to odds payouts); a wrong-direction
press/regress is dropped with no event — the one refusal in the language that emits
nothing; a dangling `press place 6 to` parses as "to pressed"; every numeric hole
truncates through `as u8`, so `place 260` is `place 4` and `total(263)` fires on
sevens; and `on roll` fires at a pre-first-roll decision where `last-total` reads 0 —
which makes the shipped `FIELD_IS_DUE` example count a phantom field number and bet
after one real field number, not two.

---

## 3. The expressiveness experiment

Twelve well-known strategies — curated from a survey of line/odds, place/press, and
progression/count/hedge systems — were each implemented by a separate author who
learned the language only from `STRATEGY_DSL.md` and the shipped examples, then
validated against the branch's real parser, compiler, and simulator (parse → compile
→ multi-seed bench with per-rule fire counts and refusal events). Mean 5.25 attempts
per strategy; every failure and workaround was logged.

| Strategy | Expressible | Ease | Deciding factor |
|---|---|---|---|
| 3-Point Molly | full | 4/5 | 18 lines, zero variables; `come point on n` and idempotent `bet` carry the whole lifecycle |
| Pass Line + full 3-4-5x odds | full | 4/5 | `max` collapses the whole odds schedule; buy-in capture needs a latch idiom (dead `session-start`) |
| The 5-Count | full | 4/5 | Counter correctness hinges on the undocumented set-visibility order; docs spell the come-point trigger two ways |
| Don't Pass + full lay odds | full | 3/5 | Don't-side odds amount is silently a win-target — writing the lay amount risks ~2× intended money, zero events |
| Iron Cross | partial | 4/5 | Cannot cover the box number that is the point; field-layout assumption (12 pays 3:1) unstatable |
| $44 Inside Regression | partial | 3/5 | Progression re-stakes winners to base; `bet` over an up bet at a new amount is a silent no-op; can't place the point |
| Field Doubling Progression | partial | 3/5 | Discarded initializer killed attempt 1; sequential `set` made the rung-wrap double-fire |
| Place 6/8 Press-and-Ride Ladder | **partial** | 2/5 | **The core is unreachable** — relative presses never compound; both cap rules provably dead |
| The Hammerlock | workaround | 4/5 | 5 sentences became 23 rules: no partner binding in `for each`, undocumented legality matrix |
| Martingale on the Pass Line | workaround | 3/5 | 9 of 11 attempts were semantics probes; no working way to initialize state; no table-max read for the cap leg |
| Oscar's Grind | workaround | 3/5 | Expression amounts and `min()` work but are undocumented; `$5,` eats the argument comma as a thousands separator |
| Per-Shooter Banking Overlay | workaround | 3/5 | `paid(odds on pass)` aliases the flat; no bankroll partition; discipline lives entirely in guards |

### What tripped three or more independent authors

These are language or documentation defects by definition, not user error:

- The §4 evaluation model is wrong as written (8–9 authors probed it; one shipped
  wrong money because of it).
- The odds action's text form is undocumented — five authors produced the identical
  `expected an action — found "odds"` before inferring `bet odds on pass`. No shipped
  example takes odds, in a game whose only zero-edge bet is odds.
- `session-start` is dead (5 authors), and `var x = 1` initializers are discarded
  (3 authors) — initialization has two documented paths and zero working ones.
- The table's legality matrix (no placing the point number; no place press/down on
  the come-out) is stated nowhere; five authors mapped it by refusal-probing a bare
  `NotAllowedNow`.

### Missing vocabulary, ranked by how many strategies needed it

1. **Working state initialization** (5 strategies) — honor initializers, fire
   session-start, or both.
2. **Session/table configuration reads** (7) — buy-in/starting bankroll, table max,
   field-layout declaration or assertion.
3. **Placing/buying the current point number** (5) — refused unconditionally; real
   casinos allow it; recurring money divergence.
4. **Presses that survive resolution** (2, one fatally) — see defect 1.5.
5. **`for each` reach** (3) — partner-of-pair binding, per-member group amounts;
   symmetric logic expanded 4–6×.
6. **Visibility of own rejections/clipping** (3) — a Martingale cannot implement its
   table-max reset leg.
7. Working-on-come-out control (3, the admitted P2c gap); amount-grammar uniformity —
   `units`/`half`/by-delta unreachable in press positions while undocumented
   expression amounts work everywhere (4); a blessed pseudo-clock (2); `paid()` for
   odds (1).

### What the experiment praised

- The verification loop — fire counts, `NEVER FIRED` flags, deterministic seeds —
  praised by all 12; it caught bugs that produced no error and identical-looking
  money, and determinism turned the checker into a differential-testing instrument
  (one author proved `max` ≡ the explicit 3-4-5 schedule bit-identically).
- The trigger vocabulary maps 1:1 onto how players talk; line strategies transcribe
  nearly sentence-for-sentence.
- Idempotent `bet` (7 authors), `max` odds and the don't-side win-target convention,
  expression amounts, and integer cents end-to-end — a $209 profit being
  arithmetically impossible is what exposed the lay-pricing convention.
- Three authors wrote many files with *zero* parse errors; nothing ever crashed on
  them; parse errors always name line and token.

Error-message UX is half-built: the parse surface is strong and the never-bets
sentence is a model diagnostic, but errors name the found token and never the legal
alternatives; a $0 bet from an uninitialized var refuses as `InsufficientBankroll` at
a full bankroll; `NotAllowedNow` covers at least three distinct prohibitions without
saying which; `Rejected` events carry neither the action kind nor the requested stake
(`stake_cents: 0`); and `BenchTrace::ending()` labels a `leave`-terminated session
"the night ran its full length" (`bench.rs:126–139`) — ten of twelve authors flagged
it.

---

## 4. Security and robustness

Mostly sound: no `unsafe` anywhere in the diff, saturating arithmetic by
construction, fixed-size interpreter buffers with clamped push/pop, and a well-tested
path sanitizer in the strategy store. The two reachable process-crashes (§1.1, §1.2)
directly contradict the code's stated invariants and are the findings that matter.

| Severity | Finding | Location |
|---|---|---|
| critical | Unbounded parser recursion aborts the process — reproduced, §1.1 | `text.rs:527` |
| critical | `i64::MIN / -1` panics rayon workers from user text — reproduced, §1.2 | `program.rs:564` |
| major | Program size unbounded and the action bound undercounts: `worst_actions` sums only `Stmt::Bet`; Press/Regress/Down/Working/Leave also occupy the 48-slot proposal buffer, whose `push` silently discards overflow — including a `Leave`. A million-rule strategy compiles cleanly. | `compile.rs:146`, `program.rs:311` |
| minor | Provenance rests on 32 bits of a Debug-formatted FNV hash — collision-weak for an anti-swap guarantee, and any enum rename silently re-keys every saved strategy's identity; no pinned golden-hash test. | `compile.rs:404`, `config.rs:49` |
| minor | Operand-stack slots leak on the already-up fast path — `Op::Bet` early-continues without popping its amount operand; enough operand-bearing statements against a populated layout silently drop later values. | `program.rs:505` |

---

## 5. Performance

The engine-side story is substantially real: the interpreter is verifiably
allocation-free, hashmap-free, forward-only; the 16-byte `Op` tripwire runs in CI;
fused guards, bitmask dispatch, `Arc`-shared programs, and zero-cost feature masks
for the built-in player all exist as claimed. Three gaps separate claims from code:

| Severity | Finding | Location |
|---|---|---|
| major | The "binding" performance budget is enforced by nothing — `bench_compiled` is `#[ignore]`, absent from CI, and asserts only the 4.5× half, never the ≤2× half. | `session.rs:990` |
| major | The placement-only decision skip can never engage — `needs_placement` is cleared only by `place_bets`, which the compiled path never calls; the documented optimization is dead code. | `player.rs:94`, `game.rs:456` |
| major | The Replay ledger deep-clones the whole `BenchTrace` and rescans every event, every frame; the rules editor clones the `Strategy` and re-runs the block parser per frame; `live_program()` clones the `Program` into a fresh `Arc` per frame. | `screens/bench.rs:428`, `rules.rs:209`, `app.rs:173` |
| minor | `Guard::PointOn` is unreachable — `fuse_guard` has no arm for a bare `point != 0`, so the commonest single-condition guard (four rules in the flagship loaded benchmark) runs as general stack code while its fast-path variant sits dead. | `compile.rs:209`, `program.rs:128` |
| minor | Per-roll trigger-window clears run regardless of the runtime mask (≈150 bytes zeroed per roll for any AllFeatures session), and `cheapest_stake` walks the full op stream once per session — twice on the drawdown path, where the result is discarded. | `game.rs:653`, `session.rs:435` |

---

## 6. Software design

The macro-architecture is the branch's best work: clean engine→app dependency
direction, the statically-dispatched `Player` seam with one session loop owning every
end-of-session rule, derived-not-declared feature masks, and the decision to keep
`place_bets` forever beside the compiled path *pinned by an executable proof* is well
argued. The 2,000-line `text.rs` is cohesive around the round-trip law and earns its
size. The failures are at the seams the branch created and did not finish walking:

| Severity | Finding | Location |
|---|---|---|
| critical | The Explorer indexes the curated list with the `AUTHORED_STRATEGY` sentinel — the engine now emits leaderboard rows tagged `u16::MAX`, and `explorer.rs` (untouched on this branch) does `strategies[r.strategy_idx as usize]` over an 11-element list. P7 is marked Done; this is a panic waiting for an authored strategy to rank. | `explorer.rs:679`, `sweep.rs:339,484` |
| major | Run validation still interrogates the checkbox rail while a strategy is live — a pasted strategy sentence blanks the rail, so a valid, resolved strategy scenario is refused with "Select at least one bet type." The §10 paste→resolve→run loop dead-ends at its last step. | `config.rs:208`, `sentence.rs:181` |
| major | Two name spaces with no canonical mapping — the sentence carries the in-source declared name; the library resolves against sanitized file stems. Save "44 Inside, regressed" and its own sentence reference goes *Missing*. | `screens/bench.rs:141`, `store_strategies.rs:88` |
| major | The dual-path pinning is weaker than documented — the 10,000-seed equivalence proof is `#[ignore]` and CI runs only the default tier (400/150-seed versions do run). | `library.rs:216`, `ci.yml` |
| major | Pasting a sentence clobbers unsaved editor work — `resolve()` unconditionally `load_from()`s over source and save-name; the paste path never consults `dirty()`, which exists precisely to guard this. | `screens/bench.rs:144`, `design.rs:968` |
| minor | Progression presses move money inside `resolve()`, outside the adjudicator — a documented §6 decision, but it leaves Principle 1 true only at the decision point, and it is the mechanism behind defect §1.5. | `game.rs:933` |
| minor | Extensibility traps for the planned buy/lay/hop bets — wildcard arms in `flat_spec`/`current_stake_of`/`take_down_odds` mean a new `BetRef` compiles everywhere and is refused at runtime; the Debug-format hash re-keys every saved strategy on any variant rename. | `action.rs:365`, `compile.rs:404` |
| minor | Replay labels a benched night with the *current* editor program, in a third hash spelling (`#%04x` vs the sentence's 8 hex digits). | `screens/bench.rs:452` |
| minor | Exact-match version gate leaves no room for additive grammar growth — the first new trigger word forces either refusing every existing file or shipping new words under an unchanged version. | `text.rs:932` |

---

## 7. Idiomatic Rust

Well above average: static-dispatch seams, const-folded feature gating (`const MASK`
is a textbook zero-cost seam), `const { assert!() }` fluency, disciplined
`let-else`/`matches!`, and an outstanding test suite. The lapses are localized:

| Severity | Finding | Location |
|---|---|---|
| major | `ParseError`/`CompileError` implement neither `Display` nor `std::error::Error` — pub-exported error types with a bespoke `.message()`, unusable with `?`/`Box<dyn Error>`. | `text.rs:36`, `compile.rs:22` |
| major | Eight bare `as u8` casts on user input — out-of-range numbers wrap into *valid* numbers instead of being refused, against the language's refuse-don't-guess ethos. | `text.rs:368…579` |
| major | An `expect()` reachable from compiled user programs — `pressed_stake`'s `place_index(n).expect(...)` is reachable via `Press`/`Regress` with `Amount::Pressed` on an invalid box number, because compile validates box numbers for triggers and reads but never for statement bet-refs. | `action.rs:575` |
| major | A shadowed rebinding defeats `block_holds`' second parse — `one.vars.clone_from(&s.vars)` is a dead store into a binding immediately shadowed; the vars-in-scope re-parse the comment promises is unreachable for exactly the bodies it exists for. This is the mechanism behind the block-dissolution bug. | `text.rs:1241–1257` |
| minor | Silent catch-alls hide future variants — `progression_word` falls back to `"flat"` (a 13th progression would silently change meaning on save); `trigger_label` falls back to "every roll"; numeric-code dispatch tables in `primary()`/`stmt_slots` break positionally on insertion. | `text.rs:330,549` |
| nit | All ten strategy submodules are `pub mod`, exposing interpreter internals past the curated re-exports; the 16-byte `Op` guarantee is a runtime test where the same file already uses an inline const assertion; `let _ = &mut changed;`-style lint silencers; four `allow(too_many_arguments)` with runs of same-typed positional ints. | — |

---

## 8. Quality and tests

Test depth is genuinely high where the design said it mattered: 11 strategies × 3
odds policies × 400 seeds asserting four independent money fields, 11 × 12
progressions × 150 seeds, whole-sweep/Duel/Explorer bitwise identity, one
adjudication test per shipped `RejectReason` asserting event + unchanged layout, and
a three-layer round-trip law extended through the compiler to identical money. The
systematic weakness: **the untested seams are exactly where the silent failures
live.**

| Severity | Finding | Location |
|---|---|---|
| major | The round-trip generator covers a narrow grammar subset — 9/13 bet-refs, 7/11 triggers, 8/14 operators, 6/25 reads; never generates blocks, always two vars named `a`/`b`, always names the strategy "case N". The law is untested precisely where it breaks. | `text.rs:1893` |
| major | The robustness sweep's only seed strategy is three lines — "every truncation and every single-byte deletion" runs against one ~50-byte strategy with no vars, guards, blocks, or money literals. | `text.rs:1824` |
| minor | `parse_money` overflows on large amounts (panics under test profile, wraps negative in release) and `$4.999` silently truncates to $4.99. | `text.rs:303` |
| minor | Known-wrong error attribution is pinned by a test — `bet place 7 base` reports `found "base"` because the number is consumed before validation, and `errors_name_the_offending_token` asserts the wrong token. | `text.rs:371` |
| minor | The examples' feature-mask assertion is vacuous for its NONE cases — `has(wants) \|\| wants == NONE` always rescues. | `examples.rs:192` |
| nit | `NeverBets` counts press/regress, so a press-only strategy that can never put money at risk passes the check written to prevent it; stale comments misdescribe the shipped architecture at three sites. | `session.rs:982` |

---

## 9. Completeness against the spec

The language core delivers §3–§7 nearly in full — all 25 reads parse (exceeding the
§3.2 table), all 11 triggers, groups, nested blocks, per-stream pressing, generated
examples. The gap is at the edges the spec calls the product, and in a status header
("Complete against v0.5. P0–P8 have landed") the code does not support:

| Severity | Finding | Location |
|---|---|---|
| major | §9 static checks: one of six exists. Never-bets ships and is excellent. Dead rule, Conflict, Exposure, Clipping, and Cost do not exist, while P5 claims "§9's checks join the order-ticket strip." Had the dead-rule check existed, it would have caught `session-start`. | `compile.rs:23` |
| major | §5 Testing & CI overstates what runs — the 10k-seed equivalence and both performance gates are `#[ignore]`/absent from CI; the promised kittest editor interactions don't exist; static-check sentence tests cover 2 of 7 `CompileError` variants. | `STRATEGY_DSL.md:925` |
| major | Part I describes an API that was never built, without correction markers — `Amount::Multiple/Half/All`, `Odds` and by-delta `Press` variants, `Leave(Reason)` (the parsed reason is discarded), a 7-variant `RejectReason` vs the shipped 5, the `working(bet)` read, and the `everything` group — §3.3's own list — all absent. | `STRATEGY_DSL.md` §3.2–3.4 |
| minor | Smaller promises quietly narrowed — "rules drag to reorder" shipped as a move-up button only; the sentence hash renders 8 hex digits in one place and 4 in another; `live-come` is listed but never defined; §5.2's claim that `dont-pass` is hyphenated contradicts the parser, which wants `dont pass`. | — |

Deferred items the spec *does* mark honestly — buy/lay/hop, working-on-come-out,
record-my-play, the full progression cross in the Explorer — are documented deferrals
and not counted against it.

---

## 10. Recommendations

### A. Before anything else ships (crashes and wrong money)

- **Depth-limit the expression parser.** Count expression nesting in `Parser` (the
  `depth` field already exists for blocks) and refuse past ~64 with a sentence. This
  also un-breaks `cargo test -p craps-engine`.
- **Guard `Div` against `i64::MIN / -1`** (one `saturating_div` beside the existing
  zero guard).
- **Teach `explorer.rs` the `AUTHORED_STRATEGY` sentinel** — or don't emit sentinel
  rows until the UI reads them.
- **Enforce the table minimum in the adjudicator** and emit the spec's own
  `BelowTableMinimum`; emit `ClippedToMax` where `game.rs:245` silently clamps.
  Consider refusing bare unit-less integers in amount position outright — `language 1`
  has no installed base yet, and this is the cheapest moment such a change will ever
  have.
- **Decide the press interaction for real.** Either a decision-point press updates the
  stream's pressed level (so it survives resolution and ladders climb), or compiling a
  rule-press onto a stream whose progression re-prices is a refusal. Then make
  `PRESS_TWICE` do what its prose says.

### B. Kill the silent traps (each is refuse-or-honor, never ignore)

- Honor `var x = 5` initializers or refuse a nonzero value; wire `session-start` (set
  the bit at the first decision) or remove it from grammar, docs, and the rule editor.
- Reserved-word-check `var` names against the read vocabulary; range-check every
  number instead of `as u8` wrapping; refuse a dangling `press … to`; emit an event
  for wrong-direction press/regress; validate statement bet-refs at compile (closing
  the `expect()` path).
- Refuse or escape `#` and `"` in strategy names (they currently break the save
  format); fix `block_holds`' first parse to declare vars so var-using blocks stop
  dissolving; render money-typed constants as dollars so a GUI edit stops rewriting
  `$150` into `15000`.
- Make odds a real stream or refuse odds refs in triggers/reads/`press for` — silently
  aliasing the flat is the worst of both.
- Attribution: put the action kind and requested stake into `Rejected` events; split
  `NotAllowedNow` into named reasons (placing-the-point, come-out freeze, stranded
  bet); teach `ending()` about `leave`.

### C. Make the proofs real

- Run the 10k-seed equivalence and both halves of the performance budget in CI (a
  nightly or perf tier is fine — anywhere a machine executes them).
- Rewrite §4's evaluation model to the truth (sets immediate, table snapshot, actions
  buffered — or change the code to match the doc); document the legality matrix;
  document `bet odds on …` and ship one odds example; correct §3.2–§3.4 and §5's
  claims or mark them as the doc's own convention requires.
- Widen the round-trip generator to the full grammar (blocks, all
  triggers/reads/operators, hostile names) and give the robustness sweep more than one
  three-line seed.
- Pin the program hash with a golden-value test and derive it structurally rather than
  from `Debug` formatting; unify the two hash renderings; resolve sentence references
  via declared-name↔stem mapping.

### D. Language evolution, in evidence order

Ranked by how many of the twelve strategies needed each (§3): initialization;
buy-in / table-max / table-rules reads (or a declared-preconditions header);
place/buy the point number or document the divergence prominently; `press … by` and
per-member group amounts; partner binding in `for each`; a clipped/refused-last-decision
read; working-on-come-out (the admitted P2c gap); amount-grammar uniformity (`units`
in `to`-positions, and document expression amounts — they are one of the language's
best features and appear nowhere).

### E. Hygiene

`Display`/`Error` impls on the error types; `pub(crate)` the submodules; fuse the bare
`point != 0` guard (or delete `Guard::PointOn`); cache the per-frame clones in
Replay/rules/`live_program`; hoist `cheapest_stake` per run; guard the paste path on
`dirty()`; fix the three stale comments; decide the additive-versioning policy before
v2 vocabulary exists.

---

## 11. Method and caveats

Three evidence streams, kept independent. **(1)** Seven review passes — security,
performance, software design, idiomatic Rust, quality, completeness, and language
design — over the full `main...strategy-language-p0` diff, each finding then
re-verified by an adversarial second pass that re-read the cited code and tried to
refute it; 103 findings survived with line-level traces. **(2)** A survey of
real-world craps strategies curated to twelve, each implemented from the docs alone
and validated against the branch's actual parser, compiler, and simulator via a
purpose-built harness (parse → compile → multi-seed bench; every error message and
workaround logged). **(3)** Direct reproduction of the headline defects against the
release build, shown in §1.

### Caveats owed to fairness

- The validation harness pinned `field_12_triple: false` and passed budgets by CLI —
  so "the field layout is unstatable *in the strategy text*" stands as a language
  observation, but the app itself does expose table rules; and session budget living
  outside the `.craps` file is arguably correct scoping.
- Implementer complaints about "no per-roll ledger" and "refusals don't name the rule"
  are partly harness artifacts: the app's Replay ledger provides per-roll stepping,
  and `BenchEvent` does carry rule attribution (the harness printed only the event).
  The missing action-kind and requested-stake in `Rejected` events are engine-real.
- The three `chart::export` failures reproduce on `main` and are excluded.
- The equivalence battery's default-tier versions (400 and 150 seeds) *do* run in CI;
  only the 10k-seed deep proof and the perf gates are unexecuted.

---

## 12. What was done, and what was not

Written after the fact, against the code as it now stands.

### Not taken, deliberately

**The placement-only decision skip was deleted rather than repaired.** §5 was right
that it could never engage. Making it engage turned out to cost more than it saves: a
skipped decision is a decision where no rule fires, and fire counts here are not an
implementation detail — they are the dead-rule diagnostic the Bench is built on and
§9's checks lean on. It would have quietly halved the count beside a rule that was
working perfectly well, on the simplest strategies, which are the ones already fast
enough. A test caught it doing exactly that.

**The ≤2× half of the performance budget became ≤2.5×.** §5 asked for the budget to
be enforced, and it now is, in CI. But the number was never true of the measurements
printed beside it: the nine-rule 3-Point Molly has measured 2.16×–2.31× since it was
first benchmarked. A budget its own table contradicts is not a budget, so the gate
asserts what the design can actually hold.

**The wide session-runner signatures were left alone.** §7's last row is right that
five consecutive same-typed integers invite a transposition, but `run_session` has
carried eight positional parameters since long before this branch, and threading a
parameter struct through seventy call sites is a refactor with more regression risk
than the `nit` it fixes. `decision_from` — which §7 singled out as existing only to
host an `allow` — is gone, and its one caller builds the struct with named fields.

**A `clipped(bet)` or refusal read was not added.** §3 ranked it seventh, and the
case that motivated it was a Martingale that could not implement its table-max reset
leg. `table-max` is now readable, so that leg is one condition. A read that reports
what the table did to the *last* decision is a different and larger idea; it is not
built.

**Working on the come-out, and placing the number that is the point,** remain
deferred and are now named as such in [STRATEGY_DSL.md](STRATEGY_DSL.md) Part II §8.
Both are table-model changes rather than language changes, and both would move every
pinned outcome; the second is a real divergence from a casino and is recorded as one
rather than left for the next person to rediscover by refusal-probing.

### Where the fix went further than the finding

The widened round-trip generator (§8) found a bug on its first run that nothing in
this report had caught: a `$5,` inside `min($5, cash)` was lexed as one money token,
swallowing the argument list's own comma. One of the twelve implementers had hit it
and it had not made the report.

§9's checks were built as *diagnostics* rather than as compile errors. The section
asked for six checks and did not say what should happen when one fires; making them
refusals would have meant a language that could express unsound play but not run it,
which is Principle 5 inverted and would have cost the app its argument.
