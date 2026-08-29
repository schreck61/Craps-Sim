# v-next — Deferred Past v0.4

This is the parking lot promised by the implementation plan
([GUI_DESIGN.md](GUI_DESIGN.md), Part II §10). Everything here was deferred
consciously, with a reason, against the non-goals contract of Part I §12.
An item leaves this list by earning a milestone of its own — never by
slipping quietly into a release.

Two items below now have a designed shape in
[STRATEGY_DSL.md](STRATEGY_DSL.md) (The Player, targeting v0.5). A shape is
not a ship: they leave this list when that document's milestones land.

- **Saved-scenario library.** The Scenario Sentence is the save format; a
  library done honestly enforces shared seeds so saved scenarios stay
  comparable, and that enforcement is design work, not scope creep. Shape
  decided in [STRATEGY_DSL.md](STRATEGY_DSL.md) §10 — a directory of
  content-hashed strategies referenced by name from the sentence, with
  hash mismatch rendering STALE. The shared-seed enforcement this item
  demands is unchanged and still required.
- **Cross-run comparison.** Permanently out per spec §12 — comparing runs
  on different dice invites exactly the false inference the Duel's common
  random numbers exist to prevent. This item never graduates.
- **CSV/data export.** v0.4 exports pixels with provenance baked in; a
  data format that can be separated from its seed, n, and scenario needs
  its own honesty rules before it ships.
- **Localization.** One maintainer, one language for now — and the
  sentence renderer/parser (the save format) makes English load-bearing,
  so translation is engine-adjacent work, not string swapping.
- **egui major-version upgrade.** 0.36 is pinned for the whole v0.4 cycle;
  the chart kernel touches only Painter/epaint, and upgrades happen
  between releases, never during one.
- **Per-bet Anchor variants beyond progressions.** The Anchor swaps
  progressions on identical dice with the handle held comparable; swapping
  bet sets changes the handle itself and needs its own caption discipline.
  Retired as stated: [STRATEGY_DSL.md](STRATEGY_DSL.md) §6 makes a
  progression a rule fragment attachable per bet stream, so per-stream
  variants get a real representation and the Anchor gains them for free.
  The caption discipline for swapping bet sets is unaffected and still owed.
- **Additional Explorer objectives.** Every objective ships with its 95%
  CI computed in-sweep, so each new one is engine surface plus a whisker
  kind — a column is the cheap part.
- **Text scaling beyond 150%.** The fluid grid reflows to 150% while
  charts keep 11 px labels by shedding gridlines; past that, legibility
  itself would have to be shed, which the spec forbids.
- **Preference persistence beyond theme + register + reduced-motion.**
  Prefs are a hand-rolled plain-text file by design (no serde); every new
  key must argue its way in.
- **Full raw retention above 15M session-slots.** Digest mode plus the
  disclosed stride sample is the permanent policy, not a stopgap —
  downsampling never happens silently, and neither does a gigabyte.
- **In-app flamegraph.** Profiling stays feature-gated and out-of-process
  (`puffin_viewer`); a dev tool does not ride in the release binary.
- **Variable-font weight axes.** Blocked on egui shaping; until then the
  six static instanced faces are the only enforceable path.
- **Windows OS-level reduced-motion query.** The in-app override exists
  everywhere and macOS reads the system setting; the
  `SystemParametersInfoW` wiring waits on a Windows machine to verify it.
- **Replay ghost overlay polish and Anchor idle-precompute breadth.** Both
  shipped in minimal form per the risk register's cut line — the ghost
  overlays one strategy plainly, and idle precompute covers the focused
  minimum only; broadening either is v-next polish, not v0.4 debt.
