# Third-Party Notices

Craps-Sim is licensed under the MIT License (see `LICENSE`).

## Algorithms

The simulation's random number generator (`src/rng.rs`) is a Rust
translation of the **xoshiro256++** generator and the **SplitMix64** seed
expander by David Blackman and Sebastiano Vigna. The authors' reference C
implementations are dedicated to the public domain under CC0 1.0; see
<https://prng.di.unimi.it/> and Blackman & Vigna, "Scrambled Linear
Pseudorandom Number Generators", ACM TOMS 2021.
Distributed binaries additionally embed the following third-party fonts via
the [egui](https://github.com/emilk/egui) GUI library's default font set
(`epaint_default_fonts`). Their license texts are included in the
`licenses/` directory and accompany every release archive.

| Font | License | Text |
| --- | --- | --- |
| Hack | MIT (c) 2018 Source Foundry Authors | `licenses/Hack-license.txt` |
| Ubuntu Light | Ubuntu Font Licence 1.0 | `licenses/Ubuntu-Font-Licence-1.0.txt` |
| Noto Emoji | SIL Open Font License 1.1 | `licenses/OFL-1.1.txt` |
| emoji-icon-font | MIT (c) 2014 John Slegers | `licenses/emoji-icon-font-MIT.txt` |

All Rust crate dependencies are used under permissive licenses
(MIT, Apache-2.0, BSD, Zlib, ISC, Unicode-3.0, BSL-1.0, or Unlicense,
individually or as dual/multi-license choices). Every release archive
includes `THIRD-PARTY-LICENSES.md`, a complete per-crate attribution file —
each compiled-in crate with its version, license, and license text —
generated for that platform's exact dependency set by
`tools/generate_attributions.py`. To reproduce it from source:

```bash
cargo install cargo-bundle-licenses --locked
python3 tools/generate_attributions.py
```
