# Package receipt — theurgy 0.3.0

| field | value |
|---|---|
| version | `0.3.0` |
| file | `theurgy.hmod` |
| bytes | 5,648,897 |
| sha256 | `72167310fa4c6845900dcf892cab8dee6f1df7f9d8878fe4404675db7878cc66` |
| source | branch `full-port` — reproducible (`rm -f Cargo.lock && cargo horizon-mod build` twice, `cmp` identical) |
| game | 26.3 |
| horizon | `0a875da91dbb0fdcb7ba8c42c11341aa229af51a` |
| horizon-mixins | `e59da9094f0c2fdda6c627fe9eff22dec6a906f0` |

## Verification of these exact bytes

- `RUSTFLAGS="-D warnings" cargo test --all-targets --offline`: **118 passed**, 0 failed, 0 warnings.
- `cargo horizon-mod check ./theurgy.hmod`: exit 0.
- `cargo horizon-mod fits ./theurgy.hmod --build 0a875da`: `0a875da  made for`, exit 0.
- **In-game verification on Horizon `0a875da` (integrated server + Vulkan GPU GUI via `horizon-ctl`)**: **34/34 scenarios + `/theurgy` commands PASS** (`all`, `interactions`, `fluids`, `sal-fluid`, `vat-interactions`, `reformation`, `flux`, `incubation-chaos`, `items`, `tools`, `divination`, `wand`, `logistics`, `logistics-advanced`, `wire-graph`, `filters`, `filter-matching`, `filter-ui`, `standard-attributes`, `special-attributes`, `attribute-filters`, `content-rules`, `worldgen`, `config-attunement`, `persistence`, `review-fixes`, `recipes`, `tooltips`, `client`, `client-features`, `book`, `item-icons`, `wire-render`, `render`).
- Not run: `cargo clippy` / `cargo fmt --check` (not bundled in the launcher Rust 1.99.1 toolchain) and Java-server `differential-*.mjs` suites (no live NeoForge Java server running). See `notes.md`.

## Changes after the 34/34 receipt (not verified in game)

This `theurgy.hmod` includes the fixes for the 0.3.0 bug list. Each one is covered by source
reading against the pinned SDK and, where it is pure logic, by a unit test (118 passing). None of
them has been run in the game yet:

- sal ammoniac bucket: `ITEM_FLUIDS` provider (tank, liquefaction cauldron); creative water/sal
  buckets and emitter clicks answer `RightClickBlock` on the client with `use_item = False`, so
  the client stops predicting a placement (needs an in-game check that the server still gets the
  click);
- mercury catalyst accepts shards and crystals (`catalysation` ingredient);
- connectors and emitters face the clicked side; nexus connections are set on placement;
- double-high apparatus: upper half delegates items and fluids to the lower half, skips its own
  ticks, breaks the lower half with a drop, and goes when its partner is removed;
- broken blocks drop with their block-entity state (`loot_drops`);
- incubator connections per side, written only on change;
- sulfur-attuned rods reference declared sulfur items (`coal`→`coals`, `cobblestone`→`normal`,
  `sand`→`colorless`, `red_sand`→`red`, `snow`→`snowball`, …); the mossy and infested rods'
  linked tags were swapped and are corrected;
- sulfur, niter and salt names are built from their source and tier (`minecraft:item_name`);
  `ru_ru` placeholders `%с` (Cyrillic) fixed to `%s`;
- sal ammoniac ore: 2–5 experience; deepslate ore hardness 4.5 without a tool requirement;
  brazier light 14 when lit, furnace-based apparatus light 0 when lit; logistics hardness 0.1;
- divination rod attuning reports why it refuses and plays the tuning fork on success;
- HUD: top-centre tooltip with the translated block name, progress and held stacks;
- Hermetica: text follows the client's language (English fallback), long words wrap by
  character, Esc in an entry goes back;
- keybinding category uses `key.category.theurgy.keybindings`; the extra R recipes key is removed
  (the recipe viewer code remains, without an entry point); numpad enter uses
  `key.keyboard.keypad.enter`;
- the same-tick take guard after an insert is per player, not global (`LAST_INSERT_TICK`).
