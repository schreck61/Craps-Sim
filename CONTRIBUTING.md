# Contributing to Craps Table Simulator

Thanks for your interest in contributing! This project welcomes bug reports,
feature requests, and pull requests.

## Getting started

You need a recent stable Rust toolchain (see `rust-version` in `Cargo.toml`
for the minimum supported version).

```bash
git clone https://github.com/jschreckengast/craps_table.git
cd craps_table
cargo run --release
```

On Linux you will need the usual GUI development packages; see the README's
Building section.

## Before you open a pull request

1. **Format**: `cargo fmt`
2. **Lint**: `cargo clippy --all-targets --release` must be warning-free.
3. **Test**: `cargo test --release` must pass. If you touch the simulation
   engine, add or update tests — payout arithmetic and house-edge convergence
   tests are the backbone of this project's correctness story.
4. Keep commits focused; one logical change per PR is ideal.

## What makes a good engine change

The simulation engine (`src/sim.rs`) aims for *casino-accurate* behavior:

- All money is integer cents; payouts must be exact, never floating point.
- New bets or rules need a source (a standard rules reference or a named
  casino variation) stated in the PR description.
- Statistical claims need a verification test: either a deterministic payout
  walk-through or a convergence test against a known house edge or a
  closed-form result (see the existing tests for both styles).
- Simulation must stay deterministic per seed: no `std::time` or global RNG
  inside session code. Seeds flow in from the caller.

## Reporting bugs

Please include:

- OS and app version (or commit hash)
- The full simulation setup (budget, bets, progression, quit rule, payout
  rules, table minimums)
- What you expected vs. what you observed; screenshots help for UI issues.

For suspected *math* bugs, a minimal dice sequence demonstrating the wrong
payout is gold — see the deterministic tests in `src/sim.rs` for the format.

## Code style

- `rustfmt` defaults; no custom style.
- Prefer plain, well-named code over cleverness. Comments explain *why* or
  cite a craps rule, not what the next line does.
- Public items in `sim.rs` carry doc comments.

## License

By contributing, you agree that your contributions will be licensed under the
MIT License that covers this project.
