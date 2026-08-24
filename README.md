# Craps-Sim

A massively parallel Monte Carlo craps simulator with a native GUI. Configure
a player — bets, bet-pressing progression, budget, quit rule, and house payout
variations — then simulate hundreds of thousands of casino sessions across all
CPU cores to see, per table minimum, how long a bankroll really lasts, where
it ends up, and what strategy best fits a goal.

Written in Rust with [egui](https://github.com/emilk/egui) for the interface
and [rayon](https://github.com/rayon-rs/rayon) for parallelism. Money is
integer cents end to end, so every payout is exact. The engine is
deterministic per seed (the test suite relies on that); each run of the app
draws a fresh seed, so repeated runs give independent samples.

## What it answers

1. **How long does a bankroll last?** For each table minimum: the
   distribution of plays (dice rolls) until the bankroll can no longer sustain
   the strategy — mean with a 95% confidence interval plus the
   5th/25th/50th/75th/95th percentiles, and the equivalent hours at a
   configurable table pace.
2. **Where does the bankroll end up?** The distribution of money in hand
   after a target session length (default 4 hours): mean, bust rate, and every
   decile P10–P90, each shown in dollars and as a multiple of the starting
   budget. Sessions that bust, or quit early while ahead, keep their actual
   ending value.
3. **How big a bankroll do you need?** The smallest budget that survives the
   target session length with a chosen confidence (default 95%). Computed
   without search error: each session is played with an unconstrained bankroll
   recording its peak cash outlay. A budget covering that peak is sufficient
   to fund the full strategy through the same dice sequence, so the confidence
   percentile of peak outlay is the recommended budget (mildly conservative:
   a smaller bankroll might scrape through on fallback base bets).
4. **What should I play?** A strategy explorer sweeps 11 classic betting
   strategies × 12 progressions × 4 quit rules per table minimum and ranks
   the combinations under a selectable objective: chance to walk out with ≥2×
   budget, chance to walk out ahead, chance to not bust, median walk-out, or
   mean walk-out. Every combination plays the same dice sequences (common
   random numbers), so head-to-head rankings are not distorted by luck, and
   re-ranking is instant — all five metrics are kept for every combination.

## Features

- **Bets**: pass line, don't pass, come and don't come (with a cap on
  concurrent bets), take/lay odds (1×–10× or 3-4-5×), field, place
  4/5/6/8/9/10, hardways, any seven, any craps.
- **Payout variations**: field 12 pays double or triple, come odds working or
  off on come-out rolls, configurable prop stake, and a table maximum that
  caps progressions the way a real table does.
- **Bet pressing**: twelve progressions — flat, full press, half press,
  press-and-pull, Paroli (3-win), 1-3-2-6, Martingale, Grand Martingale,
  D'Alembert, Reverse D'Alembert, Fibonacci, and Oscar's Grind — each bet
  stream pressing independently, with odds sized from the pressed flat and
  place/hardway winners pressed in place out of the winnings.
- **Quit while ahead**: optional take-profit rule at a multiple of the
  starting budget.
- **Performance**: hundreds of millions of dice rolls per second on a modern
  multi-core machine; each simulated session answers both the session-length
  and ending-bankroll questions in a single pass, and the default 1.2M-session
  run finishes in seconds with live progress and cancellation.

## Installing

Download your platform's file from the
[Releases](https://github.com/schreck61/Craps-Sim/releases) page:

- **macOS (Apple Silicon)**: open the `.dmg` and drag Craps-Sim to
  Applications. The app is Developer ID signed and notarized by Apple, so it
  launches without warnings.
- **Windows (x86_64)** and **Linux (x86_64)**: unpack the archive and run the
  `craps-sim` binary.

## Building from source

Requires stable Rust (see `rust-version` in `Cargo.toml`).

```bash
cargo run --release
```

On Linux, install the GUI development packages first (Debian/Ubuntu names):

```bash
sudo apt-get install libgtk-3-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev
```

## Simulation model

The engine aims for casino-accurate behavior; where a modeling choice was
required, it follows common table practice and is stated here:

- One "play" is one dice roll. Hours are computed from a configurable
  rolls-per-hour pace (default 100, a typical moderately busy table).
- Line and come bets are made at the table minimum (or the progression's
  stake); place bets are rounded to proper payout units ($6 units on 6/8, $5
  elsewhere) so payouts are exact.
- Place bets and hardways are off on come-out rolls; field and one-roll props
  work every roll; winning place and hardway bets stay up; winning come bets
  come down. The current point number is not placed.
- Come/don't come flats go down only while a point is on, travel to the number
  rolled, and stack if the number repeats; don't pass and don't come bar 12
  (push). Come odds are off on come-out by default (toggleable); don't come
  lay odds always work.
- A session ends in ruin when no bets remain on the table and cash cannot
  cover the cheapest selected bet. With a quit rule, it also ends when total
  wealth (cash plus face value of live bets) reaches the target.
- End-of-session accounting counts live bets at face value — the player takes
  down their bets and colors up.
- Each bet type runs its own independent progression stream; come and don't
  come flats share one stream each. Oscar's Grind uses its standard
  even-money sizing rule, applied to all bet types. A player who cannot
  cover the progression's stake bets the base amount instead.

## Verification

`cargo test --release` runs a suite that treats the engine as a claim to be
falsified rather than trusted:

- **House-edge convergence**: millions of simulated rolls per bet family must
  reproduce the theoretical edges — pass and come 1.41%, don't pass and
  don't come 1.36%, field 5.56% (2.78% with triple 12), place-6 2.78% per
  placement, pass + 3-4-5× odds ≈0.37% of total wagered, and Martingale
  leaving the per-dollar edge unchanged.
- **Deterministic payout walk-throughs**: exact-cent checks of naturals,
  points, odds at true odds, bar-12 pushes, come travel and stacking, lay
  sizing, press-in-place accounting, table-max clamping, and Oscar's Grind's
  never-overshoot rule.
- **Closed-form checks**: the quit-while-ahead rule reproduces the classical
  gambler's-ruin probability, and the fixed-horizon mean matches the
  expected-loss formula.
- **Boundary equivalence**: the peak-outlay statistic used for budget
  recommendations is confirmed against the ruin simulator seed by seed.

## A note on gambling

This is an educational and analytical tool. Every bet it simulates has a
negative expected value, and no betting progression changes that — the test
suite proves the house edge is identical under Martingale and flat betting.
What progressions and quit rules *do* change is the shape of the outcome
distribution, and making that shape visible is the point of this project. It
neither encourages gambling nor provides betting advice.

## Acknowledgments

The GUI embeds egui's default fonts, which include fonts licensed under the
SIL Open Font License 1.1 and the Ubuntu Font Licence 1.0; distributed
binaries include those fonts. All Rust dependencies are used under permissive
licenses (MIT/Apache-2.0 and similar).

## License

MIT — see [LICENSE](LICENSE).

## Author

Jim Schreckengast

(with AI assistance)
