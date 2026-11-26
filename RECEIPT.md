# Package receipt — theurgy 0.3.0

| field | value |
|---|---|
| version | `0.3.0` |
| file | `theurgy.hmod` |
| bytes | 5,722,393 |
| sha256 | `7f55b314ef4e5bda9b38e359a1fc46f824905166b5cdf3f1ca849346453ef74f` |
| source | branch `full-port` — reproducible (`rm -f Cargo.lock && cargo horizon-mod build` twice, `cmp` identical) |
| game | 26.3 |
| horizon | `0a875da91dbb0fdcb7ba8c42c11341aa229af51a` |
| horizon-mixins | `e59da9094f0c2fdda6c627fe9eff22dec6a906f0` |

## Verification of these exact bytes

- `RUSTFLAGS="-D warnings" cargo test --all-targets --offline`: **152 passed**, 0 failed, 0 warnings.
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

## Second bug list (rod, tanks, selection, wand, filters, tool wear, visuals)

The bytes above also carry the fixes for the second 0.3.0 bug list. The scenario suites were
not re-run. Each line below was checked on these sources in the game itself: the working tree
compiled as the game's native mod library the way the launcher's `prebuilt-api` route does
(the `rustc` call of `native/build.log`, with the launcher's generated `__horizon_entry.rs`),
installed into a copy of the launcher's `native-0a875da` game, and driven with `horizon-ctl`
(integrated server; headless for the logic, a Vulkan window for the pictures).

Checked in game:

- divination rod: the scan runs a slice per tick over the use. A full-range rod (96) with 329
  chunks loaded: 76 ticks in 3.84 s around the use (before: 2 ticks in 1.5 s); the result and
  the distance bucket are stored as before (deepslate variant found through `c:ores/*`);
- sneaking with an empty hand voids the tank of a sal ammoniac tank and an accumulator
  (held Shift; `horizon-ctl interact --sneak` does not set the sneak state the block reads);
- breaking either half of a calcination oven with a pickaxe: tool damage 1, one oven dropped,
  both halves gone;
- emitter selection: the mode line in the point's colour on every selection, nothing on a
  deselect, coloured outlines of the selected blocks while the emitter is held, the summary on
  placement ("... linked to 1 source, 1 target and 1 result pedestals", "has no target",
  "targets Calcination Oven") and "1 selected blocks removed for being out of range";
- mercurial wand: a line for every action in all five modes; select-frequency steps once per
  click on a block (it stepped twice), a left click on a block lowers it without breaking the
  block; the item HUD at the bottom left with "Set North from Input to Output";
- filter screens: the list filter (pick a stack from the shown inventory, put its ghost into
  slots, Shift-click into the first free slot, deny/allow, confirm) and the attribute filter
  (reference by Shift-click, candidates by wheel, add / add inverted, summary tooltip, confirm)
  both end with the expected components on the held filter; the server's `commit` refuses an
  unknown rule type;
- in-world HUD: "Sal Ammoniac (1000 / 2000mB)", an empty tank's line, "Mercury Flux: 0 / 500000",
  answered by the server every 5 ticks;
- sulfur and niter jars with frame, label and source item in the hand, in slots and on the ground;
- reformation trail as glowing motes in the three legs' colours; no items floating over
  pedestals;
- the floor under a distiller and a calcination oven is drawn (it was culled: the blocks did
  not declare `occludes: false`);
- Hermetica in Russian: page text keeps inside the page art on both pages; the category
  backdrop shows no seam.

Not checked in game:

- a spotlight page's item-name title (no such page was reached in the run; its helpers are
  unit-tested); the in-world HUD staying hidden under the vanilla inventory (it was seen over
  it before the `playing()` check and not looked at again after); a filter refused as a ghost
  of a filter (added after the last filter run);
- "chat drawn over the book": not reproduced. In every picture taken (category and entry
  screens, messages arriving before and while the book is open) the chat is under the book;
- the left click at nothing with the wand (`left_click_empty`), the liquefaction cauldron's
  sneak click, a remote (non-integrated) server for any of the new payloads;
- the crafting progress bar line (no recipe ran under the HUD) and the catalyst's flux line.
