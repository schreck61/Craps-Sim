# Security Policy

## Scope

Craps-Sim is a fully offline desktop application. It:

- makes **no network connections** of any kind,
- reads and writes **no files** (settings are not persisted),
- contains **no `unsafe` code** in its own sources,
- uses the system clock only to seed the simulation RNG.

The realistic attack surface is therefore limited to the supply chain
(Rust crate dependencies) and denial-of-service via pathological UI inputs.

## Supply chain

Dependencies are locked via `Cargo.lock` and consist of the egui/eframe GUI
stack, rayon, and their transitive dependencies — all under permissive
licenses. Release binaries are built from the locked dependency set by
GitHub Actions; the workflow file in `.github/workflows/` is the build
recipe of record.

## Reporting a vulnerability

If you find a security issue (for example, a dependency advisory that affects
this application, or an input that crashes or hangs the app), please open a
GitHub issue — or, if you believe the issue should not be public, email
schreck@gmail.com with the details.

You can expect an acknowledgment within a week. Since this application holds
no user data and has no privileges, most issues can be discussed publicly,
and fixes ship in the next release.
