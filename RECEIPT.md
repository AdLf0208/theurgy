# Package receipt — theurgy 0.3.0

| field | value |
|---|---|
| version | `0.3.0` |
| file | `theurgy.hmod` |
| bytes | 5,659,476 |
| sha256 | `e4fecf4912aa9f650c60dcf8261910dd1b150bfd4bf9a683f71b4eaf498e8bef` |
| source | branch `full-port` — reproducible (`rm -f Cargo.lock && cargo horizon-mod build` twice, `cmp` identical) |
| game | 26.3 |
| horizon | `0a875da91dbb0fdcb7ba8c42c11341aa229af51a` |
| horizon-mixins | `e59da9094f0c2fdda6c627fe9eff22dec6a906f0` |

## Verification of these exact bytes

- `RUSTFLAGS="-D warnings" cargo test --all-targets --offline`: **119 passed**, 0 failed, 0 warnings.
- `cargo horizon-mod check ./theurgy.hmod`: exit 0.
- `cargo horizon-mod fits ./theurgy.hmod --build 0a875da`: `0a875da  made for`, exit 0.
- **In-game verification on Horizon `0a875da` (integrated server + Vulkan GPU GUI via `horizon-ctl`)**: **34/34 scenarios + `/theurgy` commands PASS** (`all`, `interactions`, `fluids`, `sal-fluid`, `vat-interactions`, `reformation`, `flux`, `incubation-chaos`, `items`, `tools`, `divination`, `wand`, `logistics`, `logistics-advanced`, `wire-graph`, `filters`, `filter-matching`, `filter-ui`, `standard-attributes`, `special-attributes`, `attribute-filters`, `content-rules`, `worldgen`, `config-attunement`, `persistence`, `review-fixes`, `recipes`, `tooltips`, `client`, `client-features`, `book`, `item-icons`, `wire-render`, `render`).
- Not run: `cargo clippy` / `cargo fmt --check` (not bundled in the launcher Rust 1.99.1 toolchain) and Java-server `differential-*.mjs` suites (no live NeoForge Java server running). See `notes.md`.

## Changes after the 34/34 receipt

The 34/34 scenario run above is of the earlier build. This `theurgy.hmod` adds the fixes for the
0.3.0 bug list; the scenario suites were not re-run on it. Each fix below was checked by hand on
these sources in a launcher build (`horizon-launcher native-build`, integrated server, driven
with `horizon-ctl`), unless it is listed under "not checked in game".

Checked in game:

- sal ammoniac bucket fills the tank and the liquefaction cauldron, and an empty bucket takes
  sal ammoniac back out of both (`fluids::fill_empty_bucket`); nothing is poured into the world;
- creative water bucket on the accumulator: +1000 mB and no water source beside it. The client
  cancels its own click as a success for clicks the server's block answers (fluid bucket on a
  tank, an accepted input, an emitter on a target), so it no longer goes on to the item's own use;
- mercury catalyst accepts shards and crystals;
- connectors and caloric / mercury flux emitters face the clicked side, on a wall too; nexus
  connections are set on placement;
- double-high apparatus: a hopper above the upper half fills the lower half; items written
  straight into the upper half's block entity are handed down on its next tick
  (`halves::hand_down`); breaking the upper half drops one apparatus; either half goes when the
  other is removed by `/setblock`;
- broken blocks keep their state: catalyst (flux, conversion), capacitor (20560 flux), nexus (its
  id, in the same int-array form as a crafted one). `loot_drops` of a player's break runs after
  the block entity is gone, so `pre_remove_side_effects` hands the state over;
- closing a vat with a sneak click no longer shows "cannot add while closed"; an empty hand on an
  empty furnace-based apparatus no longer shows "... is not available yet";
- incubator connections per side; 12 sulfur-attuned rods craft; ore experience 2-5; deepslate ore
  hardness 4.5 without a tool; light: lit brazier 14, lit oven / distiller / cauldron 0; logistics
  hardness 0.1; divination rod messages;
- names of sulfur / niter / salt built from source and tier, mithril and stellarite included;
  `ru_ru` gained the 127 keys it lacked, 132 book texts had their link and colour markup repaired
  and 14 truncated ones were retranslated;
- HUD: top centre, scaled glyphs (lines no longer overlap), live progress, held stacks (mirrored
  to the client in `Items`), hidden behind the mod's own screens;
- Hermetica: Russian text, Esc goes back, pages shrunk to fit now shrink their glyphs too
  (`GuiDraw::Push`), text keeps clear of the spine, links render as links;
- keybinding category title, no extra recipes key.

Not checked in game:

- client prediction on a remote (non-integrated) server, where the client has no recipes: only
  fluid-bucket and emitter clicks are cancelled there;
- the tuning fork sound of a divination rod; numpad Enter in the filter editor
  (`horizon-ctl` cannot press it); the per-player same-tick take guard (needs two players); the
  Hermetica's English fallback; the mercury catalyst's own inventory is spilled on break, not
  kept in the item as upstream's loot table does.
