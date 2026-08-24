# The Long Run
## Craps-Sim GUI — Design Specification & Implementation Plan

**Status:** Proposal — nothing in this document is implemented yet. It targets the v0.4 release.

This document has two parts. **Part I** is the complete design specification for
The Long Run, a ground-up redesign of the Craps-Sim interface: its philosophy,
visual language, information architecture, every screen, every component, every
interaction, and the statistical-honesty rules the design enforces. **Part II**
is the implementation plan: the stack decision, target crate/module architecture,
engine surface changes, data and rendering layers, a milestone roadmap with
effort estimates, testing strategy, dependency budget, and risk register.
Cross-references of the form "spec §N" in Part II refer to Part I's sections.

---

# Part I — Design Specification

## 1. Design Philosophy & Principles

Seven named principles govern every decision in this document. When a future feature conflicts with one, the feature loses.

**1. No Summary Without Its Shape.** No mean, percentile, recommendation, or rank may appear unless the distribution it was cut from is on screen or one gesture away. Every hero number sits directly beneath, beside, or on top of its histogram, ECDF, or survival curve. This is the constitution; everything else is bylaws.

**2. Amber Is the Math.** Color is semantics, never mood. Amber (`#F0B429`) is reserved for epistemic statements: closed-form expectation markers, the per-dollar edge, and trust warnings (STALE, PARTIAL). Red is reserved for ruin and loss. Teal is reserved for money gained. Blue is reserved for selection and probes. No color ever moonlights.

**3. Ruin Is the Loudest Object.** On any screen where ruin appears, it must be the most visible object on that screen. The bust spike is never binned, never smoothed, never dimmed, never rendered in a subtle material. If a strategy busts often, the interface should be uncomfortable to look at. That discomfort is the product working.

**4. Nothing Moves That Didn't Happen.** Motion displays only real computed values or interpolation between real successive estimates. No count-ups, no odometer digits, no synthesized jitter, no spring overshoot (an overshooting mark momentarily displays a value that never existed), no idle animation. During streaming, bars grow because counts arrived. When nothing real is arriving, nothing moves.

**5. Two Registers, One Truth.** Every finding renders in a plain-language frequency register ("62 of 100 nights go home with less") and an exact register (mono numerals, CIs, n) — bitwise the same numbers, one gesture apart. The novice reads a sentence; the educator flips to the ledger; neither is a dumbed-down or inflated version of the other.

**6. Every Number Carries Its Provenance.** Seed, session count, engine version, and the full configuration travel with every chart, every stat, every export. The moment displayed results and current settings diverge, the results say so. A number that cannot state where it came from does not ship.

**7. One Instrument, One Maintainer.** Every component must be paintable with `egui::Painter`/epaint by a single developer and maintainable for years. No custom hit-tested table composers, no hand-rolled animation frameworks, no persistence layers in this release. When craft and feasibility conflict, we simplify the craft rather than fake it.

---

## 2. The Concept

**THE LONG RUN** — *every number in the app is a distribution you can see and touch, rendered in lamplight.*

Craps-Sim computes 1.2 million casino nights per click and today shows twelve of their percentiles in a striped grid. The Long Run's organizing move is to put the raw shapes — already sitting in memory, exact to the cent — directly under the user's cursor: survival curves you scrub, histograms whose bars are doors into real sessions, a budget line you drag through a field of a thousand dying dots, and a paired duel where two strategies live the same dice. The skin is not a dashboard's and not a casino's: a warm-dark instrument where elevation is lamp warmth instead of drop shadows, numerals are tabular mono to the cent, and one serif voice speaks the titles. The gasp is never decoration. The gasp is seeing what was always just a number — and watching it converge onto an expectation line the math drew before the first die rolled.

---

## 3. Signature Gasp Moments

Four moments survived judging. Each is specified concretely; each obeys Principle 4.

**Gasp 1 — The Rain, with the Truth drawn first.** Before the user presses Run, the final-bankroll axes are already on screen, and one object is already drawn on them: a thin amber vertical labeled `expected mean — closed form`, computed from the exact per-bet edge and the configured handle, placed before a single die rolls. The user presses Space. Bars erupt as the first sessions stream off the rayon pool; a corner counter reads `847,213 sessions · 41.2M rolls/s`; the 95% CI ribbon around the empirical mean visibly narrows as n climbs from 10⁴ to 1.2×10⁶ — and the empirical mean marker walks onto the pre-drawn amber line and stops. The run is not a progress spectacle; it is a staged, falsifiable proof that the house edge was right. Nothing is tweened beyond interpolation between successive 10 Hz snapshots of real estimates. (Engineering note, binding: restructuring the rayon sweep to batch partial `SessionOutcome`s over the mpsc channel is Milestone 1. If it slips, every chart in this document draws once at completion and the design remains whole — the Rain is an enhancement, never a dependency.)

**Gasp 2 — Dragging your budget across fate.** On the Stake screen, the peak-outlay survival curve carries a vertical amber-edged budget line the user grabs and drags. The intersection readout updates live: `$500 → survives 4 h in 83.1% of sessions… $800 → 95.2%`. Beneath the curve, a 1,000-session dot field (one dot per sampled session, positioned by its peak outlay) re-colors under the drag: dots that now die flick to ruin red, dots that now live settle to bone. Moving your hand $100 to the right and watching 120 lives change color is the moment risk-of-ruin stops being abstract.

**Gasp 3 — Same dice, two lives.** In the Explorer, select two strategies and press D. The Duel opens on a single replayed session rendered as two superimposed wealth trajectories fed identical dice — flat pass-line in its chip color, Martingale in violet, diverging roll by roll from the same 6-5, same seven-out. Beside it, the paired-difference histogram and the paired decile ladder (Section 6.7) show where across all shared sessions the two lives diverge. Watching the same dice produce a $40 loss in one life and a $2,300 crater in the other is the most honest anti-system argument ever rendered.

**Gasp 4 — The Anchor, corrected.** On the Horizon screen, a segmented control hot-swaps the pressing progression. The histogram morphs over 400 ms (each variant precomputed on the same dice — the swap is a redraw, never a fake instant re-simulation; variants not yet computed show a brief real progress fill first). Through every morph, one object never moves: the amber per-dollar edge marker, pinned, labeled `house edge: −1.41% per resolved dollar`. The expected-dollar-loss pin, by contrast, visibly drifts as the progression changes the handle — and the caption states exactly why: *"You wagered more, so you lost more. The house's cut per dollar never moved — you only changed how much you fed it."* The shape convulses; the rate is bolted down; the drift itself is the lesson.

---

## 4. Visual Language

### 4.1 Palette

Dark theme is primary ("Lamplight"); light theme is "Reading Room," for educators and export. Elevation is expressed as **surface warmth** — raised surfaces shift toward tungsten — never drop shadows in dark mode.

| Role | Dark | Light |
|---|---|---|
| Ground (app background) | `#121417` | `#F6F3EC` |
| Surface (panels, cards — one lamp closer) | `#1B1A17` | `#FFFFFF` |
| Surface-2 (drawers, popovers — closest to the lamp) | `#242019` | `#FFFFFF` + 1 px hairline |
| Hairlines & grid | `#EDE9DF` at 10% | `#22251F` at 12% |
| Primary ink (text & data) | `#EDE9DF` (bone) | `#22251F` |
| Secondary ink (axes, captions) | `#9CA3A8` | `#6B7168` |
| **Ruin / loss / bust** | `#E5484D` | `#C22D33` |
| **Ahead / gain (deliberately quieter than ruin)** | `#3EB8A5` | `#22796C` |
| **Amber — expectation & trust** (House Line, edge markers, STALE/PARTIAL) | `#F0B429` | `#9A7415` |
| Selection / probe / focus | `#58A6FF` | `#2467D6` |
| Duel identity B (paired comparisons only) | `#B48EE0` | `#7C4FB8` |
| CI bands | parent hue at 18% alpha | parent hue at 12% alpha |

**Table-minimum series — real chip colors, the sole categorical encoding for minimums,** used identically in every chart, legend, chip toggle, and table. Learned once, they carry real-world semantics for free:

| Minimum | Dark ink | Light ink | Note |
|---|---|---|---|
| $5 | `#C0574A` (brick — desaturated, never confusable with ruin `#E5484D`, which is brighter and always paired with position + label) | `#A8453A` | |
| $10 | `#5286C4` | `#3D6EA8` | |
| $15 | `#8B7BC9` | `#6F5FAE` | No canonical real chip at $15; violet is documented as ours |
| $25 | `#4C9A75` | `#357B59` | |
| $100 | `#AEB4BE` | `#4A4E55` | The black chip's data ink is its ivory edge-spot gray — a matte-black line on a dark ground is invisible, and this tool does not paint invisible data. Legend swatches draw the real chip: black disc, white edge spots. |

The five series differ in lightness as well as hue and are verified distinguishable under deuteranopia and protanopia (Section 11). Red/green as a money pairing is banned; the money pairing is red/teal.

### 4.2 Typography

Three families, all OFL-licensed and embedded in the binary:

- **Fraunces** — display voice only: screen titles (26 px, weight 500, 0.01 em tracking) and hero numbers' unit labels. Fraunces appears nowhere else. One serif signature makes the instrument a brand without costing egui anything.
- **Inter** — all UI labels and prose. 13 px body, 11 px captions and provenance lines, 15 px semibold section heads, 19 px/28 medium for Story-register lead sentences.
- **IBM Plex Mono** — every numeral in the product, tabular lining figures, slashed zero. 13 px in tables, 22 px per-chart key stats, 34 px hero numbers. Money is exact-cent, decimal-aligned, true minus sign (−), never parentheses: `$1 240.50` with thin-space thousands separators and cents at 70% alpha — the engine's integer-cents exactness displayed, not rounded away. Probabilities always carry their n: `41.2% (n = 1.2M)`. No numeral ever appears in a proportional face.

### 4.3 Spacing & Layout

4 px base unit; scale 4 / 8 / 12 / 16 / 24 / 32 / 48. Cards: 16 px padding, 8 px corner radius, 1 px hairline. Charts get 24 px breathing margins; controls pack at 8. The window is freely resizable (1100×740 minimum); the central canvas is a fluid grid built from egui's native column layouts — no hand-rolled 12-column framework. Charts own all flexible space.

### 4.4 Iconography

A 10-glyph line set, 1.5 px stroke, 16 px grid, bone ink: run, stop, pin, duel (two dots joined — this doubles as the **pair-link glyph**, stamped on any number or chart computed under common random numbers), replay, export-image, seed (hexagon around a hash), theme, ledger, chevrons. Dice pips appear in exactly one place — the Replay dice strip, where dice are data. No chip clip-art, no filled icons, no emoji.

### 4.5 Elevation & Surfaces

Three elevations, expressed as warmth (4.1). Focus and selection add a 1 px `#58A6FF` ring, never a glow. Light theme uses 2 px soft shadows in place of warmth steps, because paper has no lamp.

### 4.6 Motion

Three named durations, one easing curve:

- **`micro` = 120 ms** — hover, toggle, focus ring.
- **`structural` = 240 ms** — panel/drawer transitions, re-ranks, tab cuts.
- **`morph` = 400 ms** — histogram↔ECDF flips, progression morphs, Explorer FLIP re-sorts.
- Easing: `cubic-bezier(0.22, 1.0, 0.36, 1.0)` everywhere — objects arrive and settle like a placed chip; overshoot is forbidden on any statistical mark.
- **Live data is never eased.** Streaming charts redraw from the 10 Hz poll, interpolated to frame rate only between real successive estimates, never extrapolated.
- Numbers crossfade old→new in 120 ms. Count-ups, odometers, and digit-spinners are banned product-wide.
- OS reduced-motion collapses all tiers to 0 ms crossfades (Section 11).

### 4.7 Themes

Dark is default; toggle ⌘L; follows OS on first launch. Identical layout, identical semantics — the light theme is the same instrument printed for daylight, not a second design. Every text pair and every data ink is re-verified ≥ 4.5:1 against its ground in both themes.

---

## 5. Information Architecture & Navigation

One window. Four modes on a top segmented strip, keys 1–4:

1. **Design** — build the player.
2. **Findings** — the four core questions as four sub-tabs (Q/W/E/R) sharing one simulation run: *Endurance*, *Horizon*, *Stake*, and the tabs' shared streaming canvas.
3. **Explorer** — the 528-combo sweep, leaderboard, and the **Duel** (the app's one and only run-comparison surface, opened from within Explorer).
4. **Replay** — session theater, also reachable from any dot, bar, or percentile marker anywhere in the app.

**The Scenario Sentence** is pinned atop every mode: the entire configuration rendered as one line of prose — *"**$500** at a **$15** table · **pass + 2 comes** with **3-4-5× odds** · **half-press** · quit at **+$500 or 4 h** · field 12 pays **triple**."* Every bold fragment is a click target that jumps to and focuses that control in Design (no per-fragment popover editors — click-to-focus only). The sentence is stamped on every insight card and baked into every export. It is also the save format: ⌘C on the sentence copies it as text; pasting a sentence into Design reconstructs the configuration.

**The left rail** (visible in every mode) is a compact read-back of the current player plus the Run control, seed (hexagon glyph, click to copy, reroll button), session count, and per-minimum progress lanes during runs. Clicking the rail returns to Design.

**The status baseline** (24 px, bottom, permanent) reads, on every screen, in every state: `seed 0x9E3779B9 · n = 1 200 000/min · EDGE −1.41% per resolved $ · ≈ −$23.40/h at 100 rolls/h · v0.4.0` — the edge and dollars-per-hour figures in amber mono. The instrument never forgets what it trades.

**Movement:** 1–4 switch modes; Q/W/E/R switch Findings tabs; `[` `]` cycle the focused table minimum everywhere; selection of a minimum persists across modes. All raw arrays from the last run stay resident (~150–190 MB at 1.2M sessions × 3 metrics × 4–5 minimums — measured, acceptable, documented), so every Findings interaction is instant and Explorer re-ranking never re-simulates. The Duel re-simulates its selected pair on demand with the shared seed — milliseconds, and the UI says so.

**Staleness:** the app hashes the configuration into a fingerprint at run time. The moment any setting diverges from the fingerprint that produced on-screen results, every derived chart and stat is struck with an amber **STALE** badge (top-right of each chart frame) reading `settings changed since this run — Space re-runs`. Results and settings can never silently disagree.

---

## 6. Screen-by-Screen Specification

### 6.1 Design (player & strategy setup)

The long scrolling form dies. Layout: a central **bet rail** (60% width) and a right column (40%).

**The bet rail** is a vertical stack of bet cards — Pass, Don't Pass, Come ×N, Don't Come ×N, Field, Place 4/5/6/8/9/10, Hardways, Any Seven/Any Craps — each card carrying its controls (toggles, odds policy segmented control 1×–10×/3-4-5×, max-concurrent steppers) *and its consequence*: a per-bet **edge tick**, a small amber horizontal bar with length proportional to house edge per dollar, labeled in mono (`−1.41%`, `−9.09%`, `−16.67%`). The rail's footer sums to the **blended edge strip** — one amber bar for the whole player, recomputed live from closed forms as boxes are ticked, captioned *"Every dollar you put down costs this on average, no matter what."* Ticking Any Seven visibly lurches the strip. The cost of every checkbox is drawn at the moment of choice.

**Progression picker:** twelve cards in a 3×4 grid, each showing the progression name, a one-line plain description (today's hover documentation, preserved verbatim, now always visible), and a 60×20 px step-line sparkline of its stake sequence over a canonical W-W-L-W-L-L pattern — Martingale's exponential staircase is visible before it is chosen.

**Right column:** *Bankroll & Session* (budget, quit rule, target hours, rolls/hour), *Table* (minimums as chip toggles rendered as their real chips — `$5 $10 $15 $25 $100 +custom`), *House Rules* (field 12 double/triple, come-odds working on come-out, prop stake, table max), and an *Engine* disclosure (sessions, confidence, roll cap, seed) — power settings out of the light.

**Order-ticket validation, live:** a footer strip computes as you type — worst-case per-roll exposure at each selected minimum, and cross-field collision checks in plain words: `table max $1,000 < Martingale step 7 stake $1,280 — the progression will clip`. Impossible configurations (budget below minimum, zero bets) disable Run and say exactly why, in words, in the rail — never red outlines alone.

### 6.2 The Running Experience

Space fires the run from anywhere. The active Findings tab becomes the live canvas; every tab has a live-assembling form.

- Empty axes are already on screen, with every closed-form expectation object pre-drawn in amber — the Horizon mean line, the per-dollar edge marker. (Amber analytic lines appear **only** where the engine computes a closed form; no analytic line is ever invented for a distribution that has none.)
- Sessions stream at the 10 Hz poll: bars grow, curves unfurl, CI ribbons visibly narrow. Ribbons are painted underneath and before their point estimates, so uncertainty is experienced shrinking as evidence arrives.
- The left rail shows per-minimum lanes in chip order — `$5 ✓ · $10 ▓▓░ · $15 · $25 ·` — filling as the mpsc channel delivers; completed lanes stamp their n.
- A corner odometer reads `sessions · rolls/s` in mono. It reports; it does not spin.
- Esc cancels instantly, always, from anywhere.
- Interactions (probes, drags, tab switches) work on partial data immediately.

**Feasibility covenant (binding):** incremental streaming requires batching partial `SessionOutcome`s over the channel — Milestone 1 of the build, bounded engine surgery. Until it lands, all charts draw once per completed minimum, the amber pre-drawn expectations still stage the reveal, and nothing else in this document changes.

### 6.3 Findings · Endurance — "How long does the money last?" (Q1)

**Story register (default):** three insight cards left of the hero chart — *"A typical night at the $15 table lasts **2 h 58 m** (about 297 rolls)."* / *"5 of 100 nights are over inside **43 minutes**. Show me one ▸"* / *"5 of 100 nights outlast **7 hours**. Show me one ▸"* Each card footnotes n and the 95% CI in 11 px mono and is stamped with the Scenario Sentence.

**Hero chart — the Survival Curve.** X: dice rolls (0 → roll cap), twinned top axis in hours at the configured pace. Y: fraction of sessions still solvent, 100% → 0. One step-curve per selected minimum in its chip color, labeled at line-end (no detached legend); the focused minimum at full ink, others at 35% until hovered. P5/P25/P50/P75/P95 drop dotted lines to the x-axis with mono labels. **Censoring made visible:** sessions still solvent at the roll cap accumulate in a diagonal-hatched block at the chart's right edge labeled `survived cap — 31.2%`, with a one-line footnote: *"Capped sessions bias mean and upper percentiles low; they are counted, shown, and never dropped."* Hover raises a vertical probe reading every curve at once: `after 2.0 h: $5 table 74% alive · $25 table 31% alive`. Toggle M explodes the overlay into small multiples, one panel per minimum, shared axes.

**Ledger register (L):** the exact rolls-until-ruin percentile table per minimum — mean ± CI, all percentiles, hours-equivalent — in Plex Mono, numerically identical to the Story's claims. The toggle state is remembered per user. Tables are demoted, never deleted.

### 6.4 Findings · Horizon — "Where does the bankroll end up?" (Q2)

**Story register:** lead sentence at 19 px above the hero — *"Out of 100 nights like yours, **62 go home with less** than they brought. **9 lose everything.** 31 come out ahead."* (Integer-of-100 phrasing always carries the exact figure and CI in the Ledger; the sentence footnotes `62 = 61.8% ± 0.1`.)

**Hero chart — final-bankroll histogram,** small multiples per minimum. X: dollars, twinned axis in ×-of-budget (same scale relabeled, never two scales). Y: session count, linear (log toggle L, opt-in and labeled, because log-y visually shrinks ruin). Bins are raw counts — no kernel smoothing, no fitted density lines, ever. **The bust spike is sacred:** sessions ending at exactly $0 render as a separate ruin-red bar at the origin, physically gapped from the continuous mass, annotated `BUST 23.4%` in red mono — per Principle 3, the most visible object on the screen. Permanent verticals: hairline at 1× budget labeled `P(ahead) = 38.7%`; the amber closed-form expected-mean line (pre-drawn before the run; the empirical mean marker now sits on it); **both mean (dashed) and median (solid) markers**, so skew is legible in one glance. Beneath each panel, a **decile ladder**: ten ticks P10–P90 on a shared dollar axis, mean as an amber diamond. Toggle E morphs histogram↔ECDF over 400 ms — a real transform of the same data. Clicking any bar shows its count and offers *Enter one ▸* → Replay. Brushing an x-range reports exact probability mass: `P($200–$400) = 11.3% · 135 891 sessions`.

**The Anchor control (Gasp 4)** lives here: a segmented control of the 12 progressions above the focused panel. Swapping morphs the histogram (precomputed on the same dice — the pair-link glyph and the caption *"re-simulated on identical dice"* appear beside the control); the amber per-dollar edge marker never moves; the expected-dollar-loss pin drifts with the handle and its caption states why.

**Second chart — the wealth fan.** X: rolls (top axis hours); Y: bankroll. From a deterministic 1,000-session traced subsample: P5–P95 and P25–P75 bands with the median path and twelve faint individual trajectories. Corner badge: `sampled n = 1 000 of 1.2M` — sampling disclosed, always.

**Ledger:** the decile table P10–P90 in dollars and multiples per minimum, bust rate, mean ± CI, median.

### 6.5 Findings · Stake — "How big a bankroll do you need?" (Q3)

**Hero chart — peak-outlay survival curve (Gasp 2).** X: budget in dollars; Y: probability of surviving the target session, computed exactly from per-session peak cash outlay. One curve per minimum in chip color, small-multiple grid. Pre-marked labeled dots at 90/95/99% (`95% → $780`). Two coordinated controls: the **draggable budget line** (vertical, amber-edged, grabbable anywhere along its height) and the **confidence handle** riding the y-axis with 90/95/99 detents plus free drag — dragging either updates the other's readout live from stored percentiles, no recompute, and the instant response demonstrates the answer is exact, not searched. Dragging 95→99% shows the tail's brutal steepness; the caption prices it: *"The last 4 points of certainty cost $310."*

**The dot field:** beneath each panel, 1,000 sampled sessions as dots positioned by peak outlay, x-jittered in a shallow band, re-coloring live under the budget line — ruin red dies, bone lives.

**Hero stat:** the recommended budget at the chosen confidence, 34 px mono, with the sentence *"With **$740**, 95 of 100 nights like yours never need more."* Footnote, verbatim from the engine's contract: *"Mildly conservative: covers the worst moment of each night at full strategy."*

**Insight card:** the inverse reading — *"Your actual $500 covers **83 of 100** nights"* — connected to the curve by a dot in the selection blue.

**Ledger:** recommended-budget table, minimum × confidence.

### 6.6 Explorer — "What should I play?" (Q4)

Two coordinated panes.

**Left — the strip of 528.** One row-band per table minimum. X: the selected objective — P(≥2×), P(ahead), P(no bust), median final, mean final — switched by a segmented control. Each combo is a 5 px dot: color = strategy family (an 11-hue colorblind-verified categorical ramp reserved to this screen), shape = quit rule, ring = progression class (flat/positive/aggressive). Every dot wears a horizontal 95% CI whisker, drawn before the dot. Objective flips re-sort with a 400 ms glide — all five metrics are already in memory, and users watch "best" dissolve and reform under a different definition of best: a quiet lesson that ranking is a choice.

**Right — the leaderboard.** Top 10 expanded plus an "all 528" scroll, Plex Mono columns for all five metrics simultaneously; clicking a header re-ranks instantly. Two permanent structural features: (1) the **EDGE column** — a fixed amber per-dollar edge figure on every row, identical and negative for every progression of the same bet set — the anti-snake-oil column, never hidden, never sortable into flattery; (2) the **brass tie bracket** — rows whose objective CIs overlap the leader's are physically enclosed in a single drawn bracket (2 px, amber-tinted hairline) labeled **`Rank 1 — statistically tied · n = 50k/combo — increase sessions to separate`**. The bracket is a drawn enclosure, not a footnote; the tool refuses to crown a winner it cannot defend, and it makes that refusal a signature object. Every Explorer surface carries a permanent footer in secondary ink: *"Best of a losing field — every option here has negative expected value."*

Select one dot (click) then a second (shift-click); the pair-link glyph appears in the toolbar and **D opens the Duel.**

### 6.7 The Duel (run comparison)

The Duel is the app's only comparison surface, and it is paired-by-construction: both strategies replay the identical dice (common random numbers), re-simulated on demand for the selected pair with the shared seed (milliseconds; a real progress fill shows if it isn't). Header: A in its chip/family color, B in violet `#B48EE0`, the pair-link glyph, and the caption *"Both strategies played identical dice sequences. Differences below are strategy and residual variance — not table luck."* (The overclaim "contains no luck" is banned; CRN removes common luck, not all sampling noise.)

Four panels:

1. **Same dice, two lives (Gasp 3):** one replayed session, two superimposed step trajectories, scrubber synchronized to a shared dice strip. Quick-picks: median-gap session, biggest-gap session.
2. **The y = x scatter:** X = A's final bankroll, Y = B's, per session. Rendered as a density-binned heatmap (hexbin, bone→chip-color ramp) — never 1.2M raw circles — with the y = x diagonal etched and quadrant counts printed in the corners.
3. **The paired-difference histogram:** X = per-session Δ = A − B in dollars under identical dice; Y = count; zero line bold; teal mass right of zero, red left; hero stat `median Δ = −$12.50 (95% CI −$14.10 to −$10.90)`. When the CI on the median difference includes zero, a gray **tie band** spans it, stamped **`statistically indistinguishable at n = 50 000`** — rendered as large as any verdict would be.
4. **The paired decile ladder:** two vertical decile strips (A left, B right), P10–P90, joined rung by rung by connecting lines — showing *where along the distribution* the strategies diverge (B wins the middle, loses the left tail). The shape-versus-edge lesson made structural.

Verdict line, always present, in prose: *"B ends higher in 41.2 of 100 shared nights, lower in 44.1, within $1 in 14.7. Both have identical per-dollar expectation — the difference is shape."*

Cross-run comparison (different seeds, different dice) does not exist anywhere in the product. See Section 12.

### 6.8 Replay — Session Theater

Entered from any *Show me one ▸* / *Enter one ▸* affordance, from a session index typed directly, or from four **quick-pick chips** always visible in the Replay header: `Median night · P5 night · Best finish · Deepest drawdown`.

Layout: top, the **trajectory** — X: roll number; Y: bankroll as a step line in the session's chip color, peak outlay shaded beneath, budget line etched, ruin/quit moment marked with one red or teal tick. **Behind every trajectory, always: the full population's P10–P90 percentile envelope** at 12% alpha and the amber expected-drift line — the one is always seen inside the many, so a lucky replay visibly reads as an outlier, never a promise. Winning and ruinous sessions play with identical choreography; there is no celebration state.

Bottom: the **dice strip** — every roll as a monochrome pip-pair glyph; come-out rolls ringed, points boxed, seven-outs struck in red; a second lane lists working bets and resolutions to the cent on hover (`roll 141: place-6 hits, $35 → pressed to $60`). Transport: Space play/pause; speeds 1×/10×/60×/512× (`[` `]`); ←/→ single-roll step; scrub bar. The **ghost toggle** (G) overlays a second strategy on the same dice.

Header, always: `Night #847,201 — the 5th-percentile night · seed 0x9E3779B9 · exactly re-simulated` — copyable. The replay is not a cartoon; it is evidence, and it says so.

---

## 7. Component Library

**Chart Frame.** The universal chart container: title (Inter 15 semibold), Story sentence slot (19 px, optional), plot area, provenance corner (11 px mono: `n · seed · scenario hash`), STALE/PARTIAL badge slot (amber, top-right), export affordance. All charts live in Chart Frames; none render bare.

**Insight Card.** Surface-2 card: lead sentence (frequency register), supporting mono figures with CI, optional *Show me one ▸* affordance, Scenario Sentence stamp. Hovering a card highlights its supporting marks in the adjacent chart (bidirectional linkage).

**Probe.** Crosshair + axis flags + multi-series readout at the probe's x. 80 ms hover-in delay, instant out. Click pins (up to three, numbered, Gold tags); pinned probes persist across histogram↔ECDF toggles and appear in exports. ⌫ removes the focused pin.

**CI Whisker.** Horizontal interval mark, parent hue at full alpha, drawn *before* its point estimate (240 ms stagger on first paint) — the eye learns the interval is the finding.

**Decile Ladder.** Ten ticks P10–P90 on a shared axis + amber mean diamond + solid median tick. Used under every money histogram.

**Tie Bracket.** The drawn enclosure for statistically tied rows/blocks (Section 6.6), with its standard label format. Also used in the Duel tie band.

**Bust Bar.** The separated ruin-red origin bar with its `BUST n%` label. A single component so ruin renders identically everywhere.

**Dot Field.** The 1,000-dot sampled-session strip with live recoloring under a threshold line. Samples are deterministic per seed; the `sampled n = 1 000 of 1.2M` badge is part of the component.

**Chip Toggle.** Table-minimum selector drawn as the real chip (disc + edge spots, denomination in mono). Selection ring in blue; the chip's data-ink swatch shown beside it.

**Edge Tick / Edge Strip.** The amber per-dollar cost bar (per-bet and blended). Length linear in edge; label in mono; recomputed from closed forms live.

**Ledger Table.** Plex Mono striped grid, decimal-aligned, CIs inline, column-header re-sort. All of today's tables, restyled, numerically untouched.

**Scenario Sentence Bar.** Pinned prose read-back; bold fragments are click-to-focus targets; ⌘C copies; paste reconstructs.

**Stat Hero.** 34 px mono figure + Fraunces unit label + CI in half-size + its parent Chart Frame linkage (a hero never renders without its shape on screen — Principle 1 is enforced by the component API: `StatHero::new(value).requires(chart_id)`).

**Sparkline.** 60×20 step-line for progression stake sequences; reused in the picker and the Explorer's hover cards.

**Export.** ⌘⇧C copies the focused Chart Frame as PNG (⌘⇧S saves): rendered at 2× native resolution with title, caption, Scenario Sentence, seed, and n baked into the pixels. No image can escape its assumptions — provenance survives cropping and re-posting.

---

## 8. Interaction & Input Model

- **Hover:** raises the Probe on charts; brightens the hovered series to full ink, siblings to 35%; tooltips give the Story sentence *plus* exact mono values — hover is for reading, never just glow. Insight card ↔ chart highlighting is bidirectional.
- **Click:** selects (series, dots, rows, chips) with a blue ring; pins probes; opens drill-downs (bar → sessions behind it; double-click → Replay). Selection of a table minimum persists across all modes.
- **Shift-click:** second selection (arms the Duel).
- **Drag:** budget line and confidence handle (Stake); x-range brush on any distribution (reports exact probability mass; drag-release zooms; Esc resets); Replay scrub bar. No drag ever rearranges layout.
- **Scroll:** scrolls panels; ⌘-scroll zooms a chart's x-axis about the cursor; ⇧-scroll pans a zoomed chart; double-click resets zoom.
- **Keyboard map:** `Space` run/cancel (Replay: play/pause) · `Esc` cancel/close/reset brush · `1–4` modes · `Q/W/E/R` Findings tabs · `[` `]` cycle minimum (Replay: speed) · `L` Story/Ledger flip · `M` small multiples · `E` histogram↔ECDF · `T` show table drawer · `D` open Duel from selection · `G` ghost toggle (Replay) · `←/→` step roll (Replay) · `⌘L` theme · `⌘⇧C/⌘⇧S` export copy/save · `?` shortcut overlay. Every control is reachable by Tab in reading order with a visible 1 px blue focus ring; the mouse is optional everywhere except free-form brushing.

---

## 9. States

**First run.** No blank panels, no modal tour. Findings and Explorer show ghosted axes at 15% alpha with pre-drawn amber expectation lines and one line of copy: *"Describe a player, then press Space — 1.2 million nights, about four seconds."* A preloaded example ($500, $10 table, pass + 3-4-5× odds, 4 h) sits under a **Run the example** button so the first gasp arrives inside ten seconds of first launch.

**Streaming.** As Section 6.2: live accretion at 10 Hz, narrowing CI ribbons painted before their estimates, per-minimum lanes, mono odometer, Esc always live, partial data fully interactive. Completed minimums settle from 70% to full ink over 240 ms.

**Cancelled.** Partial results persist and render under a diagonal-hatched margin stripe with an amber banner: *"Partial run — 412 000 of 1 200 000 sessions. Intervals below are recomputed at this n; treat rankings as provisional."* CIs genuinely widen; tie brackets typically grow, and that growth is the lesson. The provenance corner records `PARTIAL · n = 412 000`. Resume re-runs cleanly (deterministic seed).

**Error.** Inline, in the panel that failed: plain-language cause (*"The roll cap is lower than the shortest possible session."*), the offending control highlighted and one click away via the Scenario Sentence, an expandable mono detail block, and **Copy diagnostic** (config + seed + version). No modal alerts, no focus traps, nothing lost from prior runs.

**Statistically tied.** Ties are structural, never typographic afterthoughts: the Explorer's tie bracket encloses the tied block with shared rank `T-1`; the Duel's tie band stamps `statistically indistinguishable` across the difference distribution at verdict size; leaderboard exports carry the bracket. Copy rules are enforced product-wide: strategies never "win" — they "go home ahead more often."

**Stale.** (Cross-cutting.) Any config edit after a run strikes every derived surface with the amber STALE badge until re-run. The Scenario Sentence shows the edited fragment underlined in amber.

---

## 10. Statistical Honesty by Design

An explicit inventory of enforcement mechanisms — each is a component or rule, not an intention:

1. **The edge is furniture.** Per-bet edge ticks at the moment of choice; the blended edge strip; the status baseline's permanent `EDGE −1.41% per resolved $ · ≈ −$23.40/h at 100 rolls/h`; the Explorer's fixed EDGE column. Dollars-per-hour at the user's own pace makes negative EV concrete, everywhere, before and after every run.
2. **Theory confronts simulation — scoped.** Closed-form expectation lines are pre-drawn in amber before the run and only where the engine computes a closed form (mean drift, per-bet edges — the quantities the test suite verifies). No analytic line is ever fabricated for survival curves or other distributions without closed forms.
3. **The corrected Anchor.** The per-dollar edge marker is pinned through progression swaps; the dollar-loss pin drifts with the handle and the caption explains the drift. The invariant we display is the invariant the test suite proves.
4. **Ruin is never smoothed.** The Bust Bar is separated, red, labeled, and by rule the most visible object on its screen. No KDE, no fitted density lines, anywhere. Log-y is opt-in and labeled.
5. **Censoring is rendered.** The hatched `survived cap` block and its bias footnote — the engine's own `censored_frac` finally surfaced. Silently truncating survivors is a small lie this UI refuses.
6. **Skew is legible.** Mean (dashed, amber diamond in ladders) and median (solid) drawn distinctly on every money distribution.
7. **Uncertainty renders first.** CI ribbons and whiskers paint before their point estimates; streaming shows them narrowing; cancellation genuinely widens them on screen.
8. **Ties refuse to flatter.** Tie brackets, T-ranks, tie bands, and the "increase sessions to separate" caption.
9. **Pairing is labeled.** The pair-link glyph marks every number computed under common random numbers; its absence teaches which comparisons would be luck-contaminated — which is why unpaired comparisons don't exist here.
10. **The one inside the many.** Every replay renders against the population's P10–P90 envelope and the amber drift line; a lucky night reads as the outlier it is. No celebration state exists in any code path.
11. **Provenance is inescapable.** Scenario Sentence on every card; seed/n/version in every chart corner; both baked into every exported pixel; STALE and PARTIAL badges close the last gaps where settings and results could silently disagree.
12. **Sampling is disclosed.** Every subsampled visual (wealth fan, dot fields) wears its `sampled n of N` badge.
13. **Frequencies over probabilities, with the exact figure attached.** "62 of 100 nights" always footnotes `61.8% ± 0.1`; the Ledger is one keystroke away and bitwise identical.
14. **Direct manipulation replaces reassurance.** The app never writes "you'll probably be fine." It hands the user the budget line and lets them drag it through the graveyard of red dots themselves.

---

## 11. Accessibility

- **Contrast:** all text ≥ 4.5:1 against its ground in both themes (WCAG AA); data inks ≥ 3:1 against chart grounds, verified per theme — including the $100 series fix (`#AEB4BE` on `#121417` ≈ 8:1; the old matte-black-on-felt failure is designed out).
- **Color-blindness:** the money pairing is red/teal, not red/green. The five chip inks and the Explorer's 11-hue family ramp are verified under simulated deuteranopia, protanopia, and tritanopia; every categorical encoding is doubled by a non-color channel (lightness ordering, line-end labels, dot shape for quit rules, ring style for progression class, hatching for censoring, position + label for ruin). No meaning is ever carried by hue alone.
- **Keyboard:** complete Tab order in reading order; visible 1 px focus rings; every action in Section 8's map; the shortcut overlay on `?`; no focus traps; no modals.
- **Screen readers:** every Chart Frame exposes its Story sentence as its accessible summary via egui's AccessKit integration; Ledger tables are native focusable grids; hero stats read value + CI + n.
- **Reduced motion:** the OS setting (and an in-app override) collapses all durations to 0 ms crossfades; streaming charts update at the raw 10 Hz cadence without interpolation; the Rain becomes discrete honest refreshes. No information is motion-only.
- **Text scaling:** the whole UI respects OS text scale up to 150%; layouts reflow via the fluid grid; charts keep minimum 11 px labels by shedding gridline density, never label legibility.

---

## 12. Explicit Non-Goals

- **No casino kitsch.** No felt textures, chip-stack illustrations, 3D dice, rolling-dice animations, or neon. The palette borrows lamplight and chip semantics as *data encodings*; it never dresses the instrument as the thing it studies. (The felt-layout table composer from the panel drafts is cut: a large custom hit-tested widget is the wrong tax on one maintainer, and the bet rail prices bets better.)
- **No sound.** A statistics instrument that makes casino noises argues against itself.
- **No celebration, streaks, badges, or gamification.** A quit-at-2× night gets a teal tick, and that is all.
- **No count-ups, odometer digits, synthesized jitter, or any motion displaying an uncomputed value.** Banned by Principle 4, without exception.
- **No pie charts, gauges, donuts, 3D, or dual independent y-scales.** Position on a common scale, always. (The $/×-budget twin axis is one measure in two units — a relabel, not a trick.)
- **No KDE, fitted density lines, or default log axes.** Smoothing hides the bust spike.
- **No single "best strategy" verdict, ever.** Rankings are objective-conditional with ties enforced structurally; the app informs, it never advises — and never uses Kelly-style prescription language or words like "ammunition."
- **No cross-run comparison and no saved-scenario library in this release.** The Duel (paired, common random numbers) is the one statistically honest comparison, and it is the only one that exists. Comparing runs on different dice invites exactly the false inference CRN prevents; the copyable Scenario Sentence is the save format for now. A library done honestly (shared seeds enforced) is v-next, not scope creep.
- **No command-line mnemonics, no Bloomberg register, no 11 px density-first tables.** The audience opens this on a Saturday; the ideas from that draft survive (STALE, mean+median, the decile ladder, order-ticket validation), its costume does not.
- **No two-lane setup, no per-fragment popover editors, no dashboard customization or theming surface beyond dark/light.** One instrument, one opinionated layout, one maintainer.
- **No cloud, accounts, telemetry, real-money hooks, or web/Electron migration.** Native egui, small binary, instant cold start, local truth. The gasp comes from the data, and the data is already in memory, in cents, exact.

---

# Part II — Implementation Plan

## 1. Stack Decision & Rationale

We stay on egui/eframe 0.36 (pinned for the whole v0.4 cycle) with rayon 1.10, and we paint every chart through a small in-house kernel built directly on `egui::Painter`/epaint — no `egui_plot`, no plotting, color-science, or animation crates. Tauri/web is disqualified by spec §12 outright; iced trades away immediate-mode probes/brushes (spec §8's per-frame cursor-coupled interactions are ten lines in a paint function here), a shipped AccessKit integration, and `egui_kittest`'s headless interaction testing — and a framework migration has no shippable midpoint, violating the milestone covenant. The spec's interactions (pinned probes surviving morphs, a draggable budget line recoloring a dot field, confidence detents, mass-reporting brushes, tie brackets, FLIP re-sorts) do not exist in `egui_plot` and would be rebuilt around it anyway; a ~1,400-line kernel we own, exercised by headless shape-golden tests, is the cheaper long-run position for one maintainer and the only way to honor Lamplight's paint-order rules (uncertainty before estimates, ruin loudest). Principle 7 names epaint as the maintainability contract; we take it literally.

## 2. Target Architecture

Two-crate Cargo workspace. The UI/engine boundary becomes a compile-time wall: `craps-engine` has zero egui dependencies, so the verification suite runs headless everywhere, and touching a screen file never recompiles the engine. This layout is the single authority; all module names below are final.

```
Cargo.toml                          # [workspace] members = ["crates/*"]
tools/subset-fonts.sh               # featfreeze → subset → rename pipeline (checked-in artifacts)
docs/v-next.md                      # parking lot for every deferral in §10
crates/
  craps-engine/                     # lib crate — deterministic, headless
    src/lib.rs
    src/rng.rs                      # Xoshiro256pp, untouched
    src/bets.rs                     # BetSelection, Rules, OddsPolicy, Progression (+ Hash/Eq derives)
    src/game.rs                     # Session<O: RollObserver = Noop>, resolve(), resolve_come_bets()
    src/session.rs                  # run_session, run_horizon_session, run_drawdown_session,
                                    # session_seed() — THE determinism contract (E1)
    src/sweep.rs                    # run_sweep (batched streaming), explorer sweep (+CI fields),
                                    # run_pair (CRN paired sweep), SweepCtl atomics
    src/trace.rs                    # RollObserver, RollEvent, BetEvent, SessionTrace,
                                    # trace_session(), WealthOnlyObserver
    src/edge.rs                     # closed forms: per_bet_edge, blended_edge, flat_drift_per_roll_cents
    src/summary.rs                  # RuinStats/HorizonStats, summarize_*, budget_for_confidence
    src/stats.rs                    # order-statistic median CI, Welford, quantile digest, hexbin binning
    tests/verification.rs           # existing 32 tests verbatim + equivalence/closed-form tests
  craps-app/                        # bin crate — everything egui
    src/main.rs                     # eframe boot (~40 lines): min_inner_size(1100×740), resizable
    src/app.rs                      # mode router, global keyboard map, rail-click → Design
    src/config.rs                   # SimConfig (single source of truth), validation, fingerprint (FNV-1a)
    src/sentence.rs                 # Scenario Sentence codec: render → spans, parse → SimConfig,
                                    # per-fragment stale diff (field-by-field vs provenance config)
    src/store.rs                    # RunStore, MinRun, Columns (SoA), SortedViews, Digests,
                                    # RunProvenance, RunStatus, AnchorCache, ExploreStore
    src/stream.rs                   # coordinator thread, collector thread (sole RunStore writer),
                                    # Duel/Anchor job queue, total-rolls & progress atomics
    src/prefs.rs                    # hand-rolled two-key plain-text prefs file (theme, register)
    src/ui/                         # theme.rs, numerals.rs, motion.rs, icons.rs
    src/chart/                      # mod.rs (ChartId, highlight registry), scale.rs (linear + opt-in log-y,
                                    #   zoom/pan window state), axis.rs, frame.rs (ChartFrame), probe.rs,
                                    #   brush.rs, marks.rs, histogram.rs, survival.rs, dotfield.rs, fan.rs,
                                    #   ladder.rs, hexbin.rs, strip.rs, trajectory.rs, dicestrip.rs,
                                    #   sparkline.rs, smallmult.rs, export.rs (offscreen 2× rasterizer)
    src/components/                 # insight_card.rs, stat_hero.rs, chip_toggle.rs, edge_strip.rs,
                                    #   ledger.rs, tie_bracket.rs, error_panel.rs, sentence_bar.rs,
                                    #   drawer.rs (T ledger drawer), odometer.rs
    src/screens/                    # design.rs, findings/{mod,endurance,horizon,stake}.rs,
                                    #   explorer.rs, duel.rs, replay.rs
    assets/fonts/                   # six subsetted, RENAMED faces + OFL license texts
    tests/                          # egui_kittest interaction tests, shape goldens, pixel snapshots
```

Responsibilities: `craps-engine` owns simulation, seed derivation, closed forms, tracing, summaries, and the batched sweep; it emits `Batch`es into a channel and knows nothing about storage. `craps-app::stream` owns the coordinator/collector threads; `store.rs` owns all resident data and every query the charts ask; `chart/` owns pixels; `screens/` compose; `components/` are the spec §7 library. `sim.rs` and `main.rs` cease to exist as monoliths.

## 3. Engine Surface Changes

All changes are flagged; nothing else in `game.rs`'s resolution logic is touched, and the existing verification suite moves verbatim and must stay green at every milestone.

**E1 — `session_seed`, the public determinism contract.** `pub fn session_seed(base: u64, min_index: u32, phase: SeedPhase, session: u64) -> u64` with `enum SeedPhase { Main = 1, Drawdown = 2, Explore = 3 }`. This replaces both of today's ad-hoc formulas (the `mode_seed` closure at main.rs:721 and the explorer formula at main.rs:631). Every consumer — sweep, Replay, Duel, Anchor CRN, dot field, wealth fan — routes through it, no exceptions. One pinned regression test asserts exact outputs for fixed inputs; the explorer's pinned-seed tests are re-pinned once, in the same commit. The batched-vs-one-shot equivalence test is stated in terms of `session_seed`.

**E2 — budgeted peak outlay.** `Session` records `budget − min(cash)` as `peak_outlay_cents` in budgeted sessions (two-line accumulator, no hot-path branch). Q3's recommendation keeps its existing unconstrained `run_drawdown_session` semantics and contract footnote verbatim. Gate: bit-identical `RuinOutcome`/`HorizonOutcome` before/after.

**E3 — resolved-handle accumulator.** `resolved_wagered_cents: i64` on `Session`, incremented at every win/loss resolution site. **Push convention, decided: pushes are excluded from resolved handle.** The don't-pass edge is −27/1925 ≈ −1.403% per resolved dollar; barred-12 come-outs, come odds returned on a come-out seven, and any future push do not increment the accumulator. The convention is documented once in `edge.rs` with every push site enumerated, and the E3 verification test — *simulated mean loss ≈ edge × simulated handle* across all 12 progressions, CI-bounded — assumes it exactly. This is the invariant Gasp 4 displays; the test suite gets stronger, not merely preserved.

**E4 — trace observer.** `Session<'a, O: RollObserver = Noop>` with empty-inline default methods (`placed`, `resolved`, `roll_end`); ~20 call sites in `resolve`/`resolve_come_bets`/`place_bets`. `Noop` monomorphizes to today's machine code — verified by the existing `reference_ruin_session` equivalence tests plus a pinned outcome vector over 10k seeds. Public entry, `trace.rs`:

```rust
pub struct RollEvent { pub roll: u32, pub dice: (u8, u8), pub point_after: Option<u8>,
    pub cash_after: i64, pub wealth_after: i64, pub events: Vec<BetEvent> }
pub struct SessionTrace { pub seed: u64, pub events: Vec<RollEvent>, pub outcome: SessionOutcomes }
pub fn trace_session(sel: &BetSelection, rules: &Rules, min_cents: i64, budget_cents: i64,
    quit_target_cents: Option<i64>, max_rolls: u64, horizon_rolls: u64, seed: u64) -> SessionTrace
```

`Vec<BetEvent>`, not SmallVec — zero new dependencies. A trace costs microseconds and ~50 KB; the wealth fan uses the stripped `WealthOnlyObserver` (one `i64` per roll). Test: `trace_session` final wealth equals `run_session`'s outcome at the same seed for 1,000 random seeds.

**E5 — closed forms, `edge.rs`.** `per_bet_edge(bet, rules)`, `blended_edge(sel, rules)`, `flat_drift_per_roll_cents(sel, rules, min_cents)` — exact rationals with derivations in doc comments: pass −7/495; don't pass −27/1925 (push-exclusive, per E3); place 6/8 −1/66; field −1/18 (−1/36 with triple 12); hardway 6/8 −1/11; any seven −1/6; odds exactly 0. Handle weights come from the point-cycle steady state (line bets resolve once per 557/165-roll cycle; field/props every roll; place bets by on-point fraction). `Rules` flows through, so the Design screen's edge strip reacts live to field-12 and odds policy. **Absorbing-barrier policy, decided:** the closed-form drift assumes an unbounded bankroll; bust at $0 truncates losses, so `E[final] > budget + drift×horizon` for any config with material bust mass. Therefore: (a) all closed-form-vs-simulation verification tests run against **unconstrained simulation** (the existing `allow_negative` path) and assert agreement within 4σ at 10⁶ sessions — legitimately exact; (b) the amber Horizon line is labeled `expected mean — closed form, unbounded bankroll`, and when the run's empirical bust fraction exceeds 2% the caption adds one line: *"Busted nights stop losing: the empirical mean sits above the drift line by $X."* The gap is taught, never hidden; (c) the first-run preloaded example is tuned to a sub-0.5%-bust configuration so Gasp 1's marker-onto-the-line moment lands visually exact. Scope guard per §10.2: the amber line draws only when `progression == Flat && quit_target.is_none()`.

**Sweep API.** `sweep.rs` adopts the channel design verbatim: `pub fn run_sweep(cfg: &SweepConfig, tx: SyncSender<Batch>, ctl: &SweepCtl)` where `SweepCtl { stop: AtomicBool, sessions_done: AtomicU64, rolls_done: AtomicU64 }`. No `ControlFlow` sink — cancellation is the shared stop atomic, checked per session. `rolls_done` accumulates total simulated rolls across both phases per batch; it is the numerator of the §6.2 odometer's rolls/s, which otherwise has no data source.

**Explorer surface.** `ExploreRow` gains 95% CI fields for **all five** objectives: Wald half-widths for the three proportions, the distribution-free order-statistic interval for median final (computed while the combo's sorted finals are in hand), and Welford mean/M2 → mean ± CI95 for mean final. Raw explorer arrays are never retained (528 × 50k × 8 B = 211 MB is out of budget; 528 digest rows ≈ 100 KB). The explorer cancel path flushes completed combos with honest partial provenance instead of today's `break 'mins` discard (main.rs:637–693). `run_pair(...) -> Vec<PairedFinal { a_cents, b_cents }>` serves the Duel under shared `session_seed(base, mi, Explore, i)`.

**Determinism & suite preservation.** The suite's subject is untouched; M1's gates are: verification suite green; batched sweep ≡ one-shot collect bitwise for pinned seeds (via `session_seed`); trace ≡ outcome; closed forms ≡ unconstrained simulation within 4σ; explicit user-visible seed replaces wall-clock entropy (main.rs:445 dies). `BetSelection`/`Rules`/`Progression`/`OddsPolicy` gain `Hash`/`Eq` derives for the fingerprint.

## 4. Data & State Layer

### Run/Scenario model

`SimConfig` (config.rs) replaces the App's loose fields: it is what the Scenario Sentence renders and parses, what workers receive, and what `fingerprint() -> u64` hashes — FNV-1a over a canonical field ordering, hand-rolled, stable across platforms and releases (no serde). `RunProvenance { seed, config, fingerprint, engine_version, sessions }` rides into every Chart Frame corner and export. `RunStatus ∈ { Streaming{n}, Complete, Partial{n}, Cancelled{n} }`. `RunStore { provenance, mins: Vec<MinRun>, status, anchor: AnchorCache, explore: Option<ExploreStore> }` behind one `Arc<Mutex<_>>`.

### Provenance & staleness

The seed lives in app state: hexagon glyph in the rail, click-to-copy, reroll. Staleness is `SimConfig::from_ui(&app).fingerprint() != store.provenance.fingerprint`, evaluated on config-widget change, one boolean driving every STALE badge. The **stale fragment underline** does not come from the boolean: the sentence renderer diffs the live `SimConfig` field-by-field against `store.provenance.config` (both already in memory) and underlines exactly the changed fragments in amber.

### Streaming snapshot pipeline (Milestone 1, binding)

One fused pass: each worker task runs `run_session` (phase `Main`) and `run_drawdown_session` (phase `Drawdown`) for its session index and emits one record.

```rust
pub struct SessionRecord {              // 25 B packed, 32 B in memory — documented as such
    pub session: u32, pub rolls: u32,
    pub final_cents: i64, pub peak_outlay_cents: i64,
    pub flags: u8,                      // CENSORED | RUIN_TARGET | BUSTED | HORIZON_TARGET
}
pub struct Batch { pub min_index: u32, pub records: Vec<SessionRecord>, pub partial: bool }
```

`BATCH = 4096` (a batch `Vec` is ~131 KB) over a bounded `sync_channel::<Batch>(256)` — backpressure by design. At ~300k fused sessions/s that is ~73 sends/s, amortized well under 0.05% overhead; an `#[ignore]`d release-mode gate asserts batched throughput ≥ 97% of a collect baseline.

**Collector — indexed writes, canonical order.** Batches arrive out of order across workers, so the collector does not append. Each minimum's SoA columns are pre-sized to `sessions` and every record is written at index `rec.session` — O(1), true session order restored, no sort. Consequences: the deterministic stride sample, Replay's `Enter one ▸` lookup, and rank→id round-trips all index canonically-ordered columns. The Welford mean/M2 fold and the 512-bin incremental histograms the collector maintains are **display-only**: intermediate 10 Hz frames are nondeterministic in which prefix they show, and we say so. On `Done` per minimum, every published statistic is **recomputed from the session-ordered columns via the existing `summarize_*` path** (which sorts first, hence order-canonical) — final numbers are bit-deterministic per seed because they are recomputed from canonical data, not because streaming aggregates are order-independent (the f64 folds are not). The collector then builds the focused minimum's sorted views, builds the per-minimum digests (three 512-bin histograms, three 1024-point exact quantile tables, counts), and the UI settles the lane from 70% to full ink.

**Cancellation** is the per-session relaxed stop check (free against a ~3 µs session). Workers flush partial batches; the collector recomputes summaries at achieved n, stamps `Partial{n}`, and CIs genuinely widen because they are recomputed. **Fallback covenant:** if streaming slips, `run_sweep` degrades to one batch per completed minimum with zero UI changes — collector and store are identical in both regimes, satisfying the spec's binding note structurally.

**Odometer:** `SweepCtl.rolls_done` (both phases) feeds `sessions · rolls/s`, rendered by `components/odometer.rs` in mono — it reports, it never spins.

### Memory budget (exact, corrected)

The app's default is **six minimums** ($5/$10/$15/$25/$50/$100 — main.rs:244); the budget is stated at that default. Policy: canonical columns are always resident for all minimums; **all three sorted views exist for the focused minimum only** and are evicted on focus change (`[`/`]` triggers a 40–70 ms collector-thread re-sort, hidden behind the 240 ms focus cut). Non-focused overlay curves (Endurance multi-min) render from the 1024-point quantile digests — visually exact at ≤800 px; the focused minimum's probes, brush counts, and budget-line drag use the full sorted views for cent-exact `partition_point` answers.

| Data (per minimum) | Layout | B/session | @1.2M | Residency |
|---|---|---|---|---|
| Canonical columns (session-ordered) | SoA: `Vec<u32>` rolls, `Vec<i64>` final, `Vec<i64>` outlay, `Vec<u8>` flags | 21 | 25.2 MB | always, every minimum |
| Sorted finals + rank→id | `Vec<i64>` + `Vec<u32>` | 12 | 14.4 MB | focused minimum only |
| Sorted outlays + rank→id | `Vec<i64>` + `Vec<u32>` | 12 | 14.4 MB | focused only |
| Sorted rolls + rank→id | `Vec<u32>` + `Vec<u32>` | 8 | 9.6 MB | focused only |
| Digests (3×hist512, 3×quantile-1024, Welford, counts) | fixed | — | ~30 KB | always |

Steady state at the six-minimum default: 6 × 25.2 + 38.4 ≈ **190 MB** — the top of the spec's documented 150–190 MB envelope; at four minimums ≈ 139 MB. `RunStore::resident_bytes()` is asserted against this table in a test so the documentation cannot rot. **Retention guardrail:** raw columns are kept in full up to 15M session-slots (2.5M sessions × six minimums); beyond, the store keeps digests plus a deterministic 100k-record stride sample, and every affected chart wears the `sampled n of N` badge — at 10M sessions × 6 minimums, full canonical columns alone would be ~1.26 GB, so digest mode engages well before it. Downsampling never happens silently. Dot-field/wealth-fan sampling is **stride sampling** (`k·n/1000`) — deterministic, uniform over i.i.d.-seeded sessions, indexing canonical columns directly.

### Precomputed variants & paired diffs

**Anchor (Gasp 4):** variants store digests only — `HorizonDigest { hist512, deciles, bust_frac, mean, mean_handle, n }`, ~5 KB each; 12 variants × 6 minimums is noise. CRN is automatic: `session_seed(base, mi, Main, i)` with only `sel.progression` changed replays identical dice (the rng feeds dice only). On run completion, an idle low-priority collector job precomputes variants for the focused minimum in picker order, cancelled instantly by any real run; an uncached variant shows the real progress fill the spec demands (~one second). Every morph is a redraw of real data.

**Duel:** app-side collector job, engine's `run_pair` at the user's explorer sessions-per-combo (500–100k, default 5k, spec captions at 50k) — index-aligned `Vec<PairedFinal>` (50k × 16 B = 800 KB, ~150 ms; 100k worst case 1.6 MB). From this one array: paired-difference histogram, quadrant counts, paired decile ladders, and quick-picks (median-gap and max-gap indices → `trace_session`, same seed, two configs). Median-Δ CI via `stats.rs::median_ci` order statistics — exact, no bootstrap. The y=x hexbin is binned once in `stats.rs::hexbin` (~10–20 ms at Duel n; the Duel never sees 1.2M pairs).

### Closed-form House Line computation

`edge.rs` per E5 above. Status-baseline semantics, decided: pre-run, or whenever the config is flat-stakes with no quit target, the permanent `≈ −$/h` figure is `flat_drift_per_roll_cents × rolls/hour`, labeled `at flat stakes`; after a run with a progression selected, it becomes `blended_edge × simulated mean handle per hour` from E3, carrying the pair-link/provenance conventions. The EDGE per-resolved-dollar figure is closed-form always — it is the invariant.

### Threading model

Four actors, one lock: UI thread (copies digests under a brief lock at 10 Hz; interactive exact queries are binary searches on the focused minimum's sorted views, ≤5 ms; never sorts, never scans another minimum's columns — `summarize_*` never runs UI-side); coordinator thread (owns the sweep, seeds, stop flag); rayon pool (pure compute, talks only through the channel and atomics); collector thread (sole `RunStore` writer; indexed batch writes, completion summaries, sorted-view builds/evictions, Anchor and Duel jobs, serialized — they never overlap a live sweep). Rejected: arc-swap snapshots (dependency and mental-model tax for contention that doesn't exist at 10 Hz).

## 5. Rendering & Chart Architecture

### Kernel

`chart/scale.rs`: `LinearScale { domain, range }` with `to_screen`/`from_screen`/`ticks()` (Wilkinson-lite), plus an **opt-in log-y flag** (labeled, per spec §6.4) and a per-`ChartId` domain-window state for zoom/pan. The $/×-of-budget twin axis is one scale with a second labeler — structurally impossible to become two scales. `ChartFrame` is the universal container (title, story slot, provenance corner, STALE/PARTIAL badge, export affordance, focus ring) driving a `ChartWidget` trait (`desired_aspect`, `paint(&mut ChartCx)`, `probe(x)`, `on_key`). `ChartCx` exposes painter, scales, theme, `StreamInterp`, and `layer(Layer)` with `Layer ∈ {Ribbon, Grid, Data, Estimate, Annotation, Overlay}` — paint-order-as-API: ribbons and whiskers can only be emitted to a layer tessellated before estimates (Honesty rule 7 enforced by types). `StatHero::new(v).requires(chart_id)` debug-panics if the chart isn't mounted (Principle 1 by API). Probe: one `ProbeState` per ChartId, 80 ms hover-in delay, up to three pins keyed by **data** x so pins survive resize, morphs, and export. Brush: data-space anchor/cursor; on release `mass_between(a,b)` answers via `partition_point` post-run or bin sums during streaming (labeled PARTIAL). **Wheel interactions land in the kernel:** ⌘-scroll zooms x about the cursor, ⇧-scroll pans, double-click resets, Esc resets the brush — all in `ChartFrame` input handling with kittest coverage. A per-screen **highlight registry** (mark-id ↔ card-id) in `chart/mod.rs` gives spec §7's bidirectional insight-card↔chart hover linkage: cards publish, chart paint reads, and vice versa.

### Per-chart technology (all epaint; 512 bins everywhere)

| Chart | File | Marks / technique |
|---|---|---|
| Final-bankroll histogram (Gasp 1) | `histogram.rs` | All 512 bins as one `Mesh` (2 tris/bin, one draw call); `BustBar` separate, gapped, `Layer::Annotation`; mean-CI ribbon in `Layer::Ribbon`; amber closed-form line (E5 label rules); mean dashed + median solid markers; **1×-budget hairline labeled `P(ahead) = …`** from `1 − partition_point(finals, budget)/n`; labeled log-y toggle |
| ECDF/survival (Endurance) | `survival.rs` | Step polyline per minimum, decimated to ≤2 vertices/px column; censoring `hatch_block` at right edge; percentile droplines; streaming draws from cumulative 512-bin digests, swaps to exact sorted view on completion |
| Peak-outlay survival + budget line + confidence handle (Gasp 2) | `survival.rs` | `BudgetLine` with ±6 px hit-band, readout `1 − partition_point(outlays, b)/n` — O(log n), the instant response *is* the exactness; `ConfidenceHandle` with 90/95/99 detents, 4 px snap |
| Dot field | `dotfield.rs` | 1,000 dots, one mesh; y-jitter `splitmix64(idx)`; color decided at paint time by `outlay ≤ budget` — recolor under drag is free repaint |
| Wealth fan | `fan.rs` | P5–P95 and P25–P75 bands as **triangle-strip meshes** (never `convex_polygon` — CI bands are concave wherever the curve bends; `convex_polygon` is reserved for hexes and single rects); median polyline; 12 faint trajectories; envelope precomputed once (1,000 `WealthOnlyObserver` traces, ~15 ms, worker-side) |
| Paired-diff histogram | `histogram.rs` reused | diverging teal/red fill, bold zero, gray tie band at verdict size when median-Δ CI includes zero |
| Decile / paired ladders | `ladder.rs` | ticks + amber mean diamond + rung-connecting lines |
| Trajectories + envelope (Gasp 3, Replay) | `trajectory.rs` | step polylines (A chip color, B `#B48EE0`); population P10–P90 envelope mesh at 12% alpha; scrub = x clip-rect. **Replay drift line, decided:** the amber analytic line draws only for closed-form configs (E5 scope guard — §6.8's "always" is read as scoped by §10.2); otherwise the envelope's center is the simulated mean path in secondary ink labeled `simulated mean (n = …)` |
| Dice strip | `dicestrip.rs` | pip pairs as circle primitives, 16 px grid, x-virtualized to the clip rect |
| Hexbin y=x | `hexbin.rs` | worker-side binning of Duel-n pairs into ~3k hexes, one mesh, bone→chip ramp, diagonal + quadrant counts |
| Explorer strip of 528 | `strip.rs` | CI whisker painted first + 5 px dot; shape by quit rule, ring by progression class; FLIP re-sort via per-dot `Tween` keyed by combo id, 400 ms. Data contract: **the five CI pairs on `ExploreRow`** (Wald ×3, order-stat median, Welford mean) — the money-metric CIs are new engine surface, computed in-sweep |
| Sparklines | `sparkline.rs` | 8-segment step-line from the `Progression` step function over canonical W-W-L-W-L-L |
| Small multiples | `smallmult.rs` | grid computing one shared scale pair across panels |

Histogram↔ECDF flips and Anchor morphs are **opacity crossfades of two complete honest renderings on shared axes** — never per-bar height interpolation (an interpolated bar is a count that never existed; Principle 4).

### Theming & typography

`Theme` is a plain token struct, every hex from spec §4.1 verbatim, `lamplight()`/`reading_room()`; charts read `Theme` only, with a thin `theme::apply(ctx)` for stock widgets. Elevation = three surface tokens; light theme swaps warmth for 2 px shadows. Six embedded faces: Fraunces 72pt Medium; Inter Regular/Medium/SemiBold (each its own `FontFamily::Name` — egui has no weight axis); IBM Plex Mono Regular/Medium for every numeral (mono is inherently tabular and decimal-alignable; egui performs no OpenType shaping, so this is also the only enforceable path). **OFL compliance, corrected:** subsetting creates Modified Versions, and IBM Plex declares the Reserved Font Name "Plex" — so `tools/subset-fonts.sh` runs `pyftfeatfreeze --features zero` (baking the slashed zero into the cmap, which pyftsubset alone cannot do), then `pyftsubset` (Latin core + U+2009, U+2212, ± × ≥ ⌘ ▸), then **rewrites every name table** to RFN-free internal names (`Longrun Display` / `Longrun Sans` / `Longrun Mono`) while retaining copyright notices; OFL texts ship in About/THIRD-PARTY-NOTICES. Checked-in artifacts are guarded by a hash test, an RFN-absence check, and the glyph-coverage test (thin space, true minus, slashed zero). Target ≈650 KB for the six files, hard ceiling 900 KB enforced by a size test. `ui/numerals.rs` owns all number formatting (`money(cents) -> LayoutJob` with thin-space thousands, true minus, 70%-alpha cents; `prob(p, n)`); a debug assertion rejects non-mono FontIds and a CI grep forbids ad-hoc money formatting elsewhere. Preferences (theme, register): hand-rolled two-key plain-text file via `prefs.rs` — consistent with the no-serde stance; eframe's serde-based persistence feature stays off.

### Motion system

`ui/motion.rs`: tokens MICRO 120 ms / STRUCTURAL 240 ms / MORPH 400 ms; `settle(t)` is a 20-line Newton solve of cubic-bezier(0.22, 1.0, 0.36, 1.0). Two disjoint channels, type-enforced: a `Tweens` registry for layout/structural motion only (drawers, FLIP, crossfades) that data marks cannot reach; statistical marks read `StreamInterp`, holding the previous and current 10 Hz snapshots and exposing only `lerp(prev, cur, alpha)` with alpha clamped to 1.0 — interpolation between real successive estimates, never extrapolation; count-ups are impossible by construction. Repaint: nothing while idle; `request_repaint_after(16 ms)` while streaming or any tween runs. Reduced motion: in-app override always; macOS via `objc2-app-kit` (already in eframe's tree) reading `accessibilityDisplayShouldReduceMotion`; Windows via `SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION)`; Linux in-app only. When set, all durations → 0 and `StreamInterp` returns raw snapshots.

### Performance strategy

Rule: the UI thread never touches a raw array during streaming; per-frame work is O(pixels). Streaming histograms accumulate worker-side (512 bins, O(1)/session; width starts at `budget×4/512` and doubles by folding adjacent bins when a value overflows — counts merge exactly). ECDFs stream from cumulative bins, swap to exact sorted arrays on completion. Precompute once, paint many (fans, hexbins, dot positions, trace subsample as immutable `Arc`s). Meshes over shapes; polylines decimated first. Culling for dice strip and the all-528 table (`ScrollArea::show_rows`). Budget: ≤2 ms paint/frame on M1, ≤6 ms on a 2015-class x86 iGPU. Profiling via the `profiling` facade + plain `puffin` behind a dev-only feature, viewed out-of-process with `puffin_viewer` — no `puffin_egui` pin (its releases lag egui and no 0.36-compatible pair is assumed). A CI smoke test asserts 4,096-session bin accumulation stays under 1 ms.

### Export

**Decided: v0.4.0 ships device-independent export.** `chart/export.rs` software-rasterizes the focused ChartFrame's epaint shape list offscreen at 2× pixels-per-point — never screen-scraping — bakes title, caption, Scenario Sentence, seed, and n into the pixels, then `png`-encodes for ⌘⇧S (via `rfd` save dialog) and `arboard`-copies for ⌘⇧C. Shape goldens cover the export path. The leaderboard mounts inside a Chart Frame so leaderboard exports exist and carry the drawn tie bracket. `ViewportCommand::Screenshot` survives only as a debug tool.

## 6. Milestone Roadmap

One budget, built bottom-up as the union of the pipeline and rendering work packages plus screen assembly. Every milestone ends green, buildable, releasable; the Ledger register keeps every v0.3.0 number visible from M2 onward.

**M0 — Workspace split (2.0 dd).** Mechanical: engine files into `craps-engine`, tests verbatim, `cargo deny` wired. Exit: `cargo test --workspace` green, behaviorally identical binary, tag v0.3.1.

**M1 — Engine surface & streaming pipeline (10.5 dd).** `session_seed` (E1) + explorer re-pin; fused `SessionRecord` batched sweep, collector with indexed writes, `RunStore`, sorted views + query API + `resident_bytes` assertions + retention guardrail; `edge.rs` (E5) with unconstrained-sim 4σ tests; E2 peak outlay; E3 handle accumulator + edge×handle invariant test; ExploreRow CI fields (all five); explorer cancel flush; `rolls_done` counter; throughput gate; visible seed. Exit: verification suite green; batched ≡ one-shot bitwise; old UI intact, releasable.

**M2 — Instrument shell & provenance (9.0 dd).** Theme (both palettes, warmth), font pipeline (featfreeze/subset/rename, hash + RFN + glyph tests), `numerals.rs`; `SimConfig` fingerprint + staleness wiring + per-fragment sentence diff; mode strip, left rail (seed copy/reroll, lanes), status baseline (with the flat/progression $/h semantics), keyboard skeleton (1–4, Q/W/E/R, `[`/`]`, Space, Esc, ⌘L, T drawer, ?), sentence render + ⌘C; Ledger restyle of today's tables; `prefs.rs`; `min_inner_size(1100×740)`, OS theme on first launch, rail-click → Design. Exit: v0.4.0-alpha1 — same information as 0.3.0, new skin.

**M3 — Chart kernel & motion (9.5 dd).** Scales (linear + labeled log-y), axes, ChartFrame, probe/pins, brush, layers, marks; ⌘-scroll zoom / ⇧-scroll pan / double-click reset; highlight registry; motion system + reduced-motion detection. Exit: kernel exercised by shape goldens and kittest probe/zoom tests.

**M4 — Findings heroes & the Rain (13.0 dd).** Horizon histogram (BustBar, CI ribbon, amber line + barrier caption, mean/median markers, P(ahead) hairline, log-y toggle on **Y** — L is taken by the register flip, deviation from §6.4 recorded here), decile ladder, E-morph; Endurance survival with censoring hatch, M small multiples; wealth fan + envelope precompute; insight cards with bidirectional highlight; odometer; streaming Rain wired to M1 batches; first-run ghosted states (Findings) + Run-the-example (bust-tuned). Exit: Gasp 1 end-to-end; alpha2.

**M5 — Stake & export (8.0 dd).** Budget line + confidence handle + dot field (Gasp 2), hero stat, recommended-budget Ledger; the offscreen 2× export pipeline (⌘⇧C/⌘⇧S, rfd, provenance baked, goldens). Exit: alpha3; export pixels carry sentence/seed/n at 2× on every platform.

**M6 — Design screen (5.5 dd).** Bet rail with edge ticks, blended edge strip, progression picker + sparklines, chip toggles, right column, Engine disclosure, order-ticket validation, sentence click-to-focus + paste-to-reconstruct. Exit: long form gone; `parse(render(cfg)) == cfg` property test over randomized configs.

**M7 — Explorer (5.0 dd).** Strip of 528 (five CI whisker kinds, FLIP), leaderboard in a Chart Frame (EDGE column, tie brackets from M1 CI fields), footer copy, Explorer ghosted first-run state, selection → Duel arming. Exit: beta1; tie-bracket golden with a synthetic tied dataset.

**M8 — Duel, Replay, Anchor (14.0 dd).** Observer generic + `trace.rs` (E4) with bit-identity tests; Duel worker + `median_ci` + hexbin; paired-diff histogram + tie band + paired ladders; trajectories + envelope + dice strip + transport + ghost toggle + quick-picks + **direct session-index entry** (mono field, validates 0..n, seeds `trace_session`); Anchor digest cache + idle precompute + real progress fill. Exit: beta2; all four gasps demonstrable.

**M9 — Hardening & release (8.5 dd).** Accessibility (contrast + CVD unit tests in CI, keyboard probes, AccessKit summaries), perf verification (frame budgets, smoke gates), CI hardening (test tiers, reduced-n closed-form checks), **error panel** (`error_panel.rs`: plain-language cause, control focus via sentence, Copy diagnostic = canonical config bytes + provenance) and cancelled/partial/error states everywhere with kittest coverage, text scaling to 150%, memory readout in Engine disclosure, CHANGELOG, v0.4.0 + DMG pipeline. Exit: ship.

**Total: 85 developer-days** (M0 2.0 + M1 10.5 + M2 9.0 + M3 9.5 + M4 13.0 + M5 8.0 + M6 5.5 + M7 5.0 + M8 14.0 + M9 8.5). At nights-and-weekends pace this is a 7–9 month arc with ten shippable checkpoints; nothing load-bears on a big-bang merge.

## 7. Testing & CI Strategy

- **Verification suite (non-negotiable):** stays in `craps-engine/tests/`, numerics untouched, headless everywhere — extended, never merely preserved: batched ≡ one-shot bitwise (via `session_seed`), trace ≡ outcome over 1,000 seeds, closed forms ≡ unconstrained simulation within 4σ at 10⁶, edge×handle ≡ mean loss across all 12 progressions (push convention baked into tolerances), pinned `session_seed` outputs.
- **Shape goldens (primary chart regression):** every chart produces its `Vec<epaint::Shape>` purely from `(data, rect, theme)`; serialized to stable text (kind, rounded coords, color) and diffed against checked-in goldens. GPU-free, platform-independent, reviewable diffs. Covers the export rasterizer path.
- **Pixel snapshots (secondary):** `egui_kittest` with `snapshot`+`wgpu`, Linux CI with Mesa lavapipe, ~10 curated full-screen shots (per screen per theme), tolerance-thresholded, Linux-only to kill driver flake.
- **Interaction tests:** kittest drives the keyboard map, STALE on edit, sentence click-to-focus, probe pinning, budget-line drag, wheel zoom/pan/reset, error-state flow — headless on all three platforms.
- **Property/unit tests:** sentence round-trip; fingerprint stability; contrast ratios ≥4.5:1 text / ≥3:1 data inks in both themes; Viénot/Brettel CVD simulation over the chip inks and 11-hue ramp (in CI — ~40 lines of pure math, not deferred); font hash + RFN-absence + glyph coverage + size ceiling; `resident_bytes` vs the §4 memory table; numerals grep lint.
- **Perf gates:** `#[ignore]`d release-mode throughput test (batched ≥97% of collect); bin-accumulation smoke (<1 ms/4,096); frame-paint microbench on demand, M9 gate on 2015-class iGPU.
- **CI:** `cargo test --workspace`, `cargo clippy -D warnings`, `cargo deny check licenses` from M0, snapshot job.

## 8. Dependency Budget

| Crate | Version | License | Why | Size impact |
|---|---|---|---|---|
| eframe / egui | 0.36 (pinned) | MIT/Apache-2.0 | shell, input, AccessKit | already paid |
| rayon | 1.10 | MIT/Apache-2.0 | engine parallelism | already paid |
| arboard | 3.6 | MIT/Apache-2.0 | image clipboard for ⌘⇧C (egui does text only) | ~250 KB |
| png | 0.18 | MIT/Apache-2.0 | export encoding (`image` is 10× for one codec) | ~150 KB |
| rfd | 0.15 | MIT | native save dialog for ⌘⇧S | ~300 KB |
| egui_kittest | 0.36 | MIT/Apache-2.0 | dev-only headless testing | 0 |
| profiling + puffin | latest compatible | MIT/Apache-2.0 | dev-only, feature-gated; viewed out-of-process | 0 (release) |
| fonts (assets) | — | OFL-1.1 ×3 | six subsetted, renamed faces | ~650 KB (ceiling 900 KB) |

Explicitly rejected: `serde`/`serde_json` (the sentence is the save format; prefs are two plain-text keys), `egui_plot` (§1), `smallvec`, `criterion`, any chart/color/animation crate. Total binary growth ≤1.4 MB; cold-start growth ≤100 ms (fonts are `include_bytes!`). `cargo deny` enforces license compatibility from M0.

## 9. Risk Register

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| 1 | Streaming refactor breaks determinism | Med | High | `session_seed` is the single pinned contract; indexed collector writes restore canonical order; finals recomputed from sorted columns; bitwise equivalence tests gate M1 |
| 2 | 85 dd overruns / maintainer burnout | High | High | Ten shippable checkpoints; §12 non-goals as contract; pre-agreed cut line if slipping: Replay ghost overlay and Anchor idle precompute (on-demand fill remains) drop to v0.4.1 |
| 3 | Memory exceeds the 190 MB envelope | Med | Med | Focused-minimum-only sorted views with eviction; digests for overlays; `resident_bytes` asserted vs the table in CI; 15M-slot retention guardrail with badges |
| 4 | Closed-form line misses empirical mean on high-bust configs | High (by math) | Med | Verification against unconstrained sim; "unbounded bankroll" label + gap caption ≥2% bust; bust-tuned first-run example |
| 5 | egui 0.36→0.37 churn | Med | Med | Pinned for v0.4; kernel touches only Painter/epaint; upgrade only between releases |
| 6 | 60 fps misses on low-end hardware | Med | Med | O(pixels) rule; single-mesh marks; worker-side binning; puffin dev profiling; M9 iGPU gate |
| 7 | OFL violation via subsetted fonts | Low | Med | Name tables rewritten to RFN-free names in the subset script; OFL texts shipped; RFN-absence test in CI |
| 8 | Pixel-snapshot flake | High | Low | Shape goldens are primary; pixels Linux-only with tolerance |
| 9 | AccessKit can't express custom charts | Med | Med | ChartFrame exposes the Story sentence as its accessible node from M4; Ledgers are native widgets; kittest AccessKit assertions |
| 10 | Export rasterizer fidelity vs on-screen render | Med | Low | Same epaint shape list feeds both; goldens diff the export path; screenshot path kept as debug cross-check |
| 11 | puffin tooling incompatible with egui 0.36 | Med | Low | `profiling` facade + out-of-process viewer; worst case, egui's built-in frame stats suffice for the budget numbers |

## 10. Deferred Past v0.4

Recorded in `docs/v-next.md`, each with its reason: saved-scenario library (the sentence is the save format; an honest library enforces shared seeds — v-next); cross-run comparison (permanently out per §12); CSV/data export (PNG only); localization; egui major-version upgrade; per-bet Anchor variants beyond progressions; additional Explorer objectives; text scaling beyond 150%; preference persistence beyond theme + register (two plain-text keys only); full raw retention above 15M session-slots (digest mode is the permanent policy, not a stopgap); in-app flamegraph UI (out-of-process profiling only); true variable-font weight axes (blocked on egui shaping); and — only if risk 2's cut line fires — the Replay ghost overlay and Anchor idle precompute, both restored in v0.4.1.
