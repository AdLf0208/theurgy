# Theurgy — port notes (slice 1: calcination)

Ported from **Theurgy 1.78.0** (`theurgy-1.21.1-neoforge-1.78.0.jar`, SHA-256
`2165c80889e24f30d3351941078247c37de238c59354eaf6bc1bb278af2bc7dd`) for
Minecraft Java 1.21.1 NeoForge, upstream `klikli-dev/theurgy` at tag
`release/v1.21.1-1.78.0` (commit `95c01bbbf9be19b21feea8b1c471e98bccea2a42`).
**MIT code / CC-BY-SA-4.0 assets** — the code is MIT (upstream's `LICENSES/MIT.txt` and the
SPDX headers); the carried assets are **CC-BY-SA-4.0**, because the licence text shipped at
the tag (`LICENSES/CC-BY-4.0.txt`) is the *Attribution-ShareAlike 4.0 International* legal
code and upstream's own README at that tag says so — the `CC-BY-4.0` in `REUSE.toml` and the
jar's `mods.toml` is a label, not terms (see the 2026-11-19 section below and
[`LICENSE-NOTICE.md`](LICENSE-NOTICE.md)). Rank 0 for the code, rank 1 for the share-alike
assets; both publishable. Local-only until the maintainer approves publishing
(ARCHITECTURE.md §8). Pin
**`1d63e0a5`** (`game.rs` `64813cf4…`), mixins **`5a64af93…`** (schema 6).

Status: **ported-partial** — slice 1 (the calcination process: the pyromantic
brazier's burn, the oven's heat-gated progress, all 86 jar recipes, the two
blocks, the five salts and their content) is implemented and verified in game.
The gap list below is the rest of the mod.

## Class + tier and the approach

- **Class:** `content`; **tier:** `deep` (`ports/ledger.csv`,
  `ports/_shared/source-map/atm10-sources.csv`). The mod's deep half is its
  GeckoLib apparatus renderers, the flux emitters' networks and the
  divination/divination-rod systems.
- **Approach** (`horizon/docs/modpacks.md`, "Tiers" and "What packs need, and
  what runs today"): a **machines-shaped slice of a deep mod** — two block
  entities with item slots, a server tick, a heat link between two blocks and a
  progress counter — carried content plus the agent port of the process. Nothing
  here is an entity, a dimension or a renderer, so nothing here is built as one.
- **Carry-over vs agent port:** the brazier's models/textures, the salt
  model/texture, the two crafting recipes, the two loot tables and the lang are
  **carried** from the jar; the block declarations, item declarations, blockstates,
  the `c:` tags, the process, the block entities, the clicks and the commands are
  the **agent port** ([`CONTENT.md`](CONTENT.md)).
- **Licence:** code MIT, carried assets CC-BY-SA-4.0 (the text shipped at the tag governs —
  see the 2026-11-19 section) → rank 0 / rank 1, publishable. Ported now because
  it is the rank-0 `content`/`deep` core mod the queue offers (`precheck-mod.py
  theurgy` → `DISPATCHABLE`), the first real alchemy machine chain in the pack,
  and the parent of `theurgy_kubejs`.
- **Why calcination** (`FINDINGS.md`): of the mod's ten processes it is the only
  one that needs no fluid, no second block entity and no item component — one
  block on a heat source, one input, one output, one counter, 86 recipes — and it
  is the first step the mod's own book teaches and the salts incubation consumes.

## What the slice does

- **`theurgy:pyromantic_brazier`** (`minecraft:copper_block` template, declared
  `lit`): one fuel slot; `PyromanticBrazierBlockEntity.tickServer` exactly — burn
  down first (`remainingLitTime -= 1`), then, only when out, load the next item's
  burn time and consume one (a crafting remainder such as a lava bucket's bucket
  replaces it). `lit = remaining > 0`, written to the block state only when it
  changes. One coal/charcoal is **1,600 ticks = 16 calcination crafts**.
- **`theurgy:calcination_oven`** (`copper_block` template, declared `lit`): input
  slot 0, output slot 1; heat is the block **below** being a lit brazier, sampled
  once every **20 game ticks** and cached (`HeatConsumerBehaviour`, including its
  own `lit` write); the progress advances **1 per heated tick**, and at the
  recipe's `time` (**every jar recipe says 100**) the result goes into the output
  and the recipe's `count` is taken from the input. Anything else — cold, no
  input, no recipe, too few items, no room for the whole result — resets the
  progress to 0 (`stopProcessing`).
- **The 86 recipes** (`data/theurgy/recipe/calcination/*.json`, generated into
  `runtime.rs`): 70 by item, 16 by tag. The ratios are the jar's —
  blackstone 1:1, packed ice 1:9, blue ice 1:64, clay 1:4, obsidian 1:2, gems 1:2,
  ores 1:3, ingots 1:2, raw materials 1:1, plant salt **2**:1 creature salt,
  dragon egg 1:5.
- **Clicks** (`TwoSlotItemHandlerBehaviour`): an item goes into the input (onto a
  stack of the same item); an empty hand takes the output first and the input
  second; the brazier takes fuel and gives a stored item back.
- **`/theurgy info <pos>`**, **`/theurgy recipe <item>`**, **`/theurgy recipes`**:
  readouts for players and the harness (upstream has no commands; precedent
  `/charginggadgets info`, `/powah info`).

## Upstream files read (behaviour only; the Rust is a clean-room rewrite)

`src/main/java/com/klikli_dev/theurgy/`:

| File | What it settled |
|---|---|
| `content/recipe/CalcinationRecipe.java` | `DEFAULT_TIME = 100`, `SizedIngredient` + result + time, `matches` by ingredient |
| `content/apparatus/calcinationoven/CalcinationOvenBlockEntity.java`, `…CraftingBehaviour.java`, `…StorageBehaviour.java`, `…CachedCheck.java` | the tick's `(heated, hasInput)`, the two slots, `PreventInsertWrapper`, the output-room rule, `getIngredientCount` |
| `content/behaviour/crafting/CraftingBehaviour.java` | `progress++` per heated tick, reset to 0 otherwise, `extractItem(0, count)`, `insertItemStacked(…, true)` |
| `content/behaviour/heat/HeatConsumerBehaviour.java` | the 20-tick heat cache and the `lit` state write |
| `content/behaviour/itemhandler/TwoSlotItemHandlerBehaviour.java` | output-then-input on an empty hand, insert with the remainder left in hand |
| `content/apparatus/pyromanticbrazier/PyromanticBrazierBlockEntity.java`, `…Block.java` | burn-down-then-refuel, the crafting remainder, the `lit` state, the click |
| `registry/BlockRegistry.java`, `registry/CapabilityRegistry.java` | `strength(1.0f)`, `noOcclusion`, `SoundType.COPPER`, METAL map colour, the brazier's light 14, the brazier's item handler on every side |
| `data/theurgy/recipe/calcination/*.json` (86), `data/theurgy/tags/item/*.json`, `data/theurgy/loot_table/blocks/*`, `assets/theurgy/lang/en_us.json` | every number in the table, the tags and the content |
| `…/modonomicon/books/the_hermetica/entries/apparatus/{calcination_oven,pyromantic_brazier}.json` + the lang text | that the oven sits **on top of** the brazier and that a hopper may feed either |

## Hooks (all stamped; no pending hook is named)

| Hook (API) | Helper | What it does |
|---|---|---|
| `block_entity` (15) | `block_entity.rs` | both blocks get a block entity |
| `block_entity_ticks` (15) | `block_entity_ticks.rs` | both tick on the server |
| `block_entity_container` (16) | `block_entity_container.rs` | oven: input 0 / output 1, fills reach 0, drains take 0 and 1; brazier: one slot both ways |
| `block_entity_tick` (17) | `block_entity_tick.rs` | the oven's heat + progress, the brazier's burn |
| `use_item_on` (13) | `use_on.rs` | insert with an item |
| `right_click_block` (21) | `right_click.rs` | take with an empty hand |
| `register_commands` (8) | `register_commands.rs` | the three `/theurgy` usages |
| `perform_command` (8) | `perform_command.rs` | their bodies |
| `content_root` (raw target) | `embedded_content.rs` | writes the embedded `.hmod` out and appends it to the dev list |

9 injections (8 named hook points + one raw `content_root` target), 2 modules,
**0 unstamped**.

## Pin facts that shaped this slice

1. **`use_item_on` (API 13) does not run for an empty hand** — `Game::use_hand`
   returns before the hook when `held()` is `None` — so upstream's empty-hand take
   lives in `right_click_block` (API 21), which hears every right click on a block.
   Found in game: the first e2e took nothing until the second hook was added.
2. **A programmatic click can deliver two uses in one tick.** `ctl interact`'s
   press and release can both start a use (vanilla's 4-tick use delay makes that
   impossible for a player), so an insert could be undone by the second click's
   empty-hand take. The port records the tick an insert happened in and skips the
   take in that same tick (a deviation, below); the e2e also feeds fuel through
   the container path, which is what the mod's own tooltip says a hopper does.
3. **The engine's item tags answer a native hook** at this pin
   (`native_server_item_in_tag`, API 63, and the server's merged tags), so the
   runtime keeps the jar's tag names and the content declares the `c:` tags 26.3
   lacks; a tag ingredient works in game (e2e phase E2: one diamond → two
   Mineral Salts through `#c:gems`).
4. **A block's block entity may exist on any template** (API 129), so both blocks
   sit on `minecraft:copper_block` — a full cube with no vanilla block entity of
   its own — and keep their own states and entities.
5. **The `heatCache`/`lit` write inside a tick works** (`native_set_block_property`,
   API 18); the block entity survives it.

## Deviations (deliberate, recorded)

1. **One block, not two.** Upstream's oven and brazier are `half=lower|upper`
   2-block structures with the entity in the lower half. The port ships each as a
   single block (as `waystones` did). The furnace-style model of the oven is a
   vanilla placeholder (`minecraft:block/furnace` / `furnace_on`): upstream draws
   it with a GeckoLib block-entity renderer, so its jar block model holds only a
   particle texture. The brazier's model is upstream's own.
2. **No per-state light.** The content schema's `light` is constant, so the
   brazier's lit-light 14 cannot follow `lit`; both blocks declare 0 (as unlit).
3. **The input slot accepts any item.** Upstream's `InputInventory.isItemValid`
   refuses an item no recipe takes; the port lets it in and lets the tick refuse
   to process it. The click hook runs on the client too, where the engine's item
   tags answer `NotServer`, so a tag-gated insert would refuse every gem, ore and
   log on that side.
4. **A same-tick take is skipped after an insert** (pin fact 2). No player can
   click twice in one tick, so this changes nothing in real play.
5. **No tooltips, no GUI, no animation.** Upstream's item/block tooltips (a big
   part of its guidance), its GeckoLib bobbing/ring animations, sounds and
   particles are not carried or written.
6. **`theurgy:alchemical_salt` is named plainly.** Upstream has no lang key for
   the generic salt; the four typed ones are upstream's `%s` template + `.source`
   expanded ("Alchemical Salt Strata", …).
7. **The oven's loot table drops from any state.** Upstream's table drops only
   from `half=lower`, which a single block does not have.

## Gaps (the rest of the mod)

- **The other nine processes**: distillation (mercury shards, 45 recipes),
  liquefaction (sulfurs, 358, needs the sal ammoniac fluid), fermentation (286),
  digestion (31), incubation (287), reformation (631), accumulation (4),
  catalysation (2) — with their machines, fluids and menus.
- **The other items**: 287 alchemical sulfurs, 19 niters, mercury shard/crystal,
  purified gold, jars, divination rods (4 tiers × 4 attunements), filters,
  ceramic jars, sal ammoniac crystal/bucket.
- **The apparatus not carried**: distiller, liquefaction cauldron, fermentation
  and digestion vats, incubator (+3 vessels), the three reformation pedestals,
  sal ammoniac accumulator/tank, mercury catalyst, caloric and sulfuric flux
  emitters, the six logistics nodes, copper wire, sal ammoniac ores, crystallized
  water/lava.
- **Heat beyond the brazier**: the Caloric Flux Emitter (remote heat over 8
  blocks, redstone-controlled) and the Capability-based heat receiver.
- **Client half**: GeckoLib renderers (the oven's animated body, the lit rings),
  sounds, particles, the JEI/EMI category pages, Jade, the Hermetica book (100
  Modonomicon pages).
- **Advancements** (9) and the two block tags the jar declares only for its ores.
- **Modded tag members**: only vanilla members exist in 26.3, so a tag ingredient
  cannot match another mod's ingot/ore/log (`#c:ingots` has the four vanilla
  ingots, not Mekanism's).
- **Hoppers pulling from the oven's input** are allowed (upstream's combined
  wrapper allows it too); a hopper can also insert an item the tick will not
  process (deviation 3).

## Verification

| Step | Result | Evidence |
|---|---|---|
| `cargo test` (standalone, red) | `1 passed; 16 failed` — every engine function `unimplemented!` | `ports/_shared/evidence/theurgy/cargo-test-red.log` |
| `cargo test` (standalone, green) | **17 passed; 0 failed** | `…/cargo-test-green.log` |
| `cargo clippy --all-targets -- -D warnings` | exit 0 | `…/cargo-clippy.log` |
| `cargo fmt --check` | clean | run in session |
| `horizon.py stage` | exit 0, `stage_changed: true` | `…/horizon-check.log` (same lease family) |
| `horizon.py check` | `cargo_exit 0`, **0 warnings** | `…/horizon-check.log` |
| `horizon.py test` (embedded) | **17 passed**, 1240 filtered | `…/horizon-test.log` |
| `horizon.py build` (fast) | `cargo_exit 0` | the e2e's own rebuild |
| `cargo horizon-mod build` / `check` | `.hmod` **22,938 B**, SHA-256 `c2d085d69189cb629fd1139495cb32abcc7fbc52e8dcc23a858057a75c2d8318`, 51 entries, "no code: content only" | `…/package-hmod.log` |
| `tools/package.py` (twice) | `.hnmod` **33,517 B**, SHA-256 `9ff2af0fc32aa199021c9bc197627cc6d98f458e378eb2d73be90fd25b523638`, 14 entries, byte-identical on a second run | `…/package-hnmod.log` |
| `cargo horizon-mod check theurgy.hnmod --kind native-mod` | `native-mod`, format `native-1/1d63e0a…/5a64af9…`, for 26.3 | `…/check-native.log` |
| `tools/e2e.sh` (default) | **PASS**, `EXIT:0` — 8 phases: cold control, heat, 1:1 craft, take, 1→9 ratio, `#c:gems` tag 1→2, counted 2→1, stick's 100 ticks | `…/e2e-default.log` |
| `tools/e2e.sh` `SELFTEST=1` | **FAIL**, `EXIT:1` (asserts 8 instead of 9) — the scenario can fail | `…/e2e-selftest.log` |
| `tools/e2e.sh` `EMBEDDED=1 CONTROL=1` | **PASS**, `EXIT:0` with no `--dev-mod`; the control (unusable `.horizon-cache`) has no oven item and the block id is unknown; the written file is byte-identical to `content/theurgy.hmod` | `…/e2e-embedded.log` |
| `python3 ports/tools/check-e2e-logs.py theurgy --strict` | `logs=3 ok=3 missing-exit=0 bad-exit=0` | run in session |
| `python3 ports/tools/check-skills.py --notes ports/theurgy` | `SKILLS GATE: PASS` | run in session |
| `python3 ports/tools/upstream_ready.py theurgy` | exit 1, **one blocker only**: `status is pending, not ported` — the ledger row is the orchestrator's to write (the brief reserves `ports/ledger.csv`), and every other gate it reads passes: the jar licence is reviewable (MIT, rank 0), `theurgy.hnmod` sits in the port root, `notes.md` has the `Skills:` line, an e2e log ends `PASS` + `EXIT:0`, and `compatibility.json` pins `1d63e0a5…` with `game.rs` `64813cf4…` | run in session |

Skills: superpowers/using-superpowers,brainstorming,writing-plans,test-driven-development,systematic-debugging,verification-before-completion; BeautifulRust/rust-best-practices,rust-port

## Review repairs (PR #637, 2026-11-11)

Review: `ports/_shared/horizon-mods/reviews/pr-637-theurgy.md` (CHANGES_REQUESTED, 2 items).

| Reviewer's item | This job |
|---|---|
| 1. relative link `(klikli-dev/theurgy)` | was already absolute; re-checked |
| 2. "clarify the licensing before presenting the whole project as MIT" — root LICENSE CC-BY-4.0, LICENSE-MIT MIT, LICENSE-NOTICE not saying which files each covers | **fixed**: the allocation now comes from upstream's own `REUSE.toml` (code MIT with `SPDX-License-Identifier: MIT` headers; `src/main/resources/**`, templates and generated resources CC-BY-4.0; the Gradle wrapper Apache-2.0 and not carried). The publish tree's root `LICENSE` is the port's MIT text, `LICENSE-CC-BY-4.0` keeps the asset licence, `LICENSE-NOTICE.md` is the allocation table, and the registry says `license = "MIT AND CC-BY-4.0"` |

Upstream also mislabels `LICENSES/CC-BY-4.0.txt` (it contains the ShareAlike text); the
notice records that and ships the declared Attribution 4.0 legal code instead.

Artifact: `theurgy.hnmod` **51,021 B** `596ae21ac2cd2e0e244f26d12acad3a719d7799f2d37335d01c6edde9a49bf48`
(content `theurgy.hmod` **37,072 B**, licence files included).

## Migration to the SDK format (2026-11-18)

Migrated from the retired `.hnmod` container (Horizon's mod-API merge #16941, launcher
≥ 0.2.70; PR #17992 cleans up the old files) to the flat SDK package at nightly
**`0b14f8acfd97fa2c750e42795249dcc981178ca4`** / mixins
**`e59da9094f0c2fdda6c627fe9eff22dec6a906f0`**, launcher 0.2.78. This is the port's **first
publication** — there is no closed PR to supersede. The tables above are the 0.1.0 record
(they describe the retired container); everything below is 0.2.0.

**Done in this order:** the old layout was moved into
`../_shared/evidence/theurgy/retired-hnmod-0.1.0/` **before** the first
`cargo horizon-mod build` (the tool refuses a root holding retired package paths), then
`src/runtime.rs` was copied with `cp` and proved with `cmp` + `sha256sum`, `cargo test` ran on
the new crate **before any glue existed** (17 passed), and only then were `src/lib.rs` and
`src/support.rs` written.

### File map

| Old `.hnmod` layout | New SDK package |
|---|---|
| `horizon-mixin.toml` (`[mod]`, `[compatibility]`, 2 `[[module]]`, 9 `[[injection]]`) | `horizon-mod.toml` (id/name/version/api/side/capabilities + `[native]` game/horizon/mixins) |
| `Cargo.toml` (`[lib] path = "mixins/theurgy/runtime.rs"`, `[workspace]`) | `Cargo.toml` (one dependency: `horizon-mod-api` from `.horizon/sdk/`) |
| `mixins/theurgy/runtime.rs` (module + crate lib) | `src/runtime.rs` — **byte-identical**, `pub mod runtime;` at the crate root |
| `mixins/theurgy/support.rs` (`theurgy_state` module, 552 lines) | `src/support.rs` (the ticks, the clicks, the readouts) |
| 9 per-hook helper `.rs` files | methods of `impl Hooks` or of a Part, or a `Registrar` call (table below) |
| `content/` (`assets/`, `data/`, its own `horizon-mod.toml`, `theurgy.hmod`) | package root: `assets/`, `data/` — the content half of the same `.hmod` |
| `mixins/theurgy/data/theurgy-content.hmod` + `embedded_content.rs` (the AE2 write-out) | **gone**: code and content are one package, so there is nothing to embed |
| `compatibility.json`, `rust-toolchain.toml`, `horizon-native.json` | **gone**: `[native]` in `horizon-mod.toml` is the pin; the launcher's toolchain builds |
| `tools/horizon.py` (stage/check/test/build), `tools/package.py`/`package.sh`, `tools/notice.py`, `PLAN.md` | **gone** (moved to the evidence folder); the SDK ladder is `cargo test` + `cargo horizon-mod build/check/fits` |
| `theurgy.hnmod` (51,021 B) + `content/theurgy.hmod` (31,738 B) | `theurgy.hmod` one file (60,047 B, 61 entries) |

**Kept and updated:** `tools/gen_recipes.py` (now writes `src/runtime.rs`),
`tools/assemble_content.py` (now writes `assets/` + `data/` at the package root and no longer
writes a content-only manifest), `tools/parse_info.py` (docstring path), `tools/e2e.sh`,
`FINDINGS.md`, `CONTENT.md`, `README.md` (rewritten for the flat package), `AGENTS.md`,
`LICENSE*`, this file.

**Retired into evidence:** `mixins/`, `theurgy.hnmod`,
`content/horizon-mod.toml`, `content/theurgy.hmod`, `content/target/`, `compatibility.json`,
`rust-toolchain.toml`, `Cargo.lock`, `PLAN.md`, `tools/horizon.py`, `tools/package.py`,
`tools/package.sh`, `tools/notice.py` — all under
`../_shared/evidence/theurgy/retired-hnmod-0.1.0/`.

### Helper → hook table

| Old `[[injection]]` / helper | New place | Mapping notes |
|---|---|---|
| `[[module]] runtime` (`theurgy`) | `pub mod runtime` | the file is byte-identical; `cargo test` runs its 17 tests |
| `[[module]] state` (`theurgy_state`) | `pub mod support` | the game-side glue, now Part host calls |
| `block_entity` (`block_entity.rs`) | `BlockEntityPart::block_entity` on `Oven`/`Brazier` | upstream created an empty compound; a declared type starts with one, and the Part answers `Some(Compound::new())` |
| `block_entity_ticks` (`block_entity_ticks.rs`) | `BlockEntityPart::block_entity_ticks` | `event.ticks = true` on each Part |
| `block_entity_container` (`block_entity_container.rs`, built a `ModContainer`) | `Registrar::block_container(&block, …)` in `Hooks::register` | a declaration, not a hook: oven 2 slots (fill input 0, drain 0 and 1), brazier 1 slot both ways |
| `block_entity_tick` (`block_entity_tick.rs`, bindings `game`/`pos`/`entity`) | `BlockEntityPart::block_entity_tick` | `event.pos`, `event.entity.nbt`/`.items` (the checked view, saved because the reply stays `Changed`) |
| `use_item_on` (`use_on.rs`) | `BlockPart::use_item_on` | insert from the hand through `ItemPlace::PlayerHand` in a transaction with `set_item_slot` |
| `right_click_block` (`right_click.rs`, the empty hand) | `BlockPart::use_without_item` | the retired pin needed `right_click_block` only because API 13's `use_item_on` never runs for an empty hand; the SDK has the empty-hand method, so the take is where upstream puts it |
| `register_commands` (`register_commands.rs`) | `Hooks::register_commands` (`RegisterCommandsEvent.commands`) | same three usages |
| `perform_command` (`perform_command.rs`, string words) | `Hooks::run_mod_command` (`CommandInvocation`) | typed arguments: `<pos:block_pos>` → `BlockPos([i32;3])`, `<item:item>` → `Item(ItemStack)`; `Flow::Cancel` = handled; the readout text is byte-identical |
| `embedded-content` (`content_root` HEAD) | **none** | one flat `.hmod` carries `assets/` + `data/` itself |

Host calls that replaced the pin's `Game` helpers: `game.native_block(pos)` →
`level.block(host, pos).id(host)`; `game.native_block_property` →
`level.block_property(host, pos, key)`; `game.native_set_block_property` →
`host.set_block_property(pos, key, value)`; `game.world.game_time` →
`level.game_time(host)`; `game.native_server_item_in_tag` →
`host.tags().key(..)` + `Tags::is_in_tag`; `providers.burn_time` →
`host.item_burn_time(item)`; `items().get(..).remainder` →
`host.item_crafting_remainder(stack)`; `native_take_items` + `native_give_stack` →
`ItemPlace::PlayerHand::{extract,insert}` and `PlayerRef::give_stack`; `native_feedback` →
`host.commands().feedback(Text::literal(..))`; `native_with_mod_block_entity` →
`host.block_entity(level, pos)` → `item_slot` / `set_item_slot`.

### Manifest mapping

- identity: `id = "theurgy"` (also the resource namespace), `name`, `version = "0.2.0"`
  (the port's version, not upstream's 1.78.0), `license = "MIT AND CC-BY-4.0"` in
  `Cargo.toml`, `authors = ["klikli-dev (upstream)", "Horizon native mod contributors"]`.
- `api = "0.2"` (exact), `side = "both"` (an integrated server belongs to the client
  installation), `capabilities = ["registries"]` (the schema's one value; this package does
  declare blocks, items and block entity types).
- `[native] game = "26.3"`, `horizon = "0b14f8a…"`, `mixins = "e59da90…"` replace
  `[compatibility] horizon_revision` and the bundle's mixins revision.
- `[mod] priority = 100` and the plan's two modules are gone; nothing in this slice needed
  ordering (`#[listener(priority = …)]` is the new lever).

### Build and verify (measured 2026-11-18)

Every command ran in a `ports/tools/build-slot.sh` lease with `BUILD_LOCAL=1` (job
`theurgy-sdk`; steps `test`, `lint`, `fmt`, `build`, `check`), directly in the port —
`build-slot.sh` speaks the retired `tools/horizon.py` and is used here as the slot guard, not
as the driver. `CARGO_INCREMENTAL=0`, no `--target-dir`.

| Step | Command | Result | Evidence |
|---|---|---|---|
| Runtime tests, before any glue | `cargo test --offline` with only `src/runtime.rs` wired | **17 passed**, 0 failed | `evidence/sdk-test-runtime-only.log` |
| Glue's first run (red) | same, with the helpers as `todo!()` | **18 passed; 5 failed** — `not yet implemented` | `evidence/sdk-test-glue-red.log` |
| Runtime + glue | `cargo test --offline` | **23 passed**, 0 failed (17 runtime + 6 glue) | `evidence/sdk-test.log` |
| Lints | `cargo clippy --all-targets --offline -- -D warnings` | exit 0, 0 warnings | `evidence/sdk-clippy.log` |
| Format | `cargo fmt --check` | clean; `src/runtime.rs` unchanged by `cargo fmt` (same sha256 before/after) | run in session |
| Package | `rm -f Cargo.lock && cargo horizon-mod build` | `target/horizon-mod/theurgy.hmod`, exit 0 | `evidence/sdk-build.log` |
| Reproducible | the same command a second time | same **60,047 B**, same sha256 `bab2b93c…` | `evidence/sdk-build-second.log` |
| Registry check | `cargo horizon-mod check theurgy.hmod` | `theurgy 0.2.0 (native-mod, format native-1/0b14f8ac…/e59da909…, for 26.3)`, **60,047 B**, sha256 `bab2b93c…`, **exit 0** | `evidence/sdk-check.log` |
| Fits the pinned build | `cargo horizon-mod fits theurgy.hmod --build nightly` | `0b14f8a  made for`, exit 0 | `evidence/sdk-fits.log` |
| `runtime.rs` untouched | `cmp` against the retired file **and** `unzip -p theurgy.hmod src/runtime.rs \| sha256sum` | `15d3ab29…3ae7` both, byte for byte | session |
| Carried content | every `assets/`+`data/` entry against the retired 0.1.0 build | **50 files, 0 differ, 0 missing** | `evidence/sdk-runtime-carry.log` |
| What it uses | `unzip -p theurgy.hmod uses.json` | 346 entries: 24 hook, 39 method, 28 trait, 167 type, 78 handle, 5 part, 2 macro, 2 fn, 1 capability | `evidence/sdk-uses.json`, `evidence/sdk-uses-summary.txt` |

`check` output:

```
theurgy 0.2.0 (native-mod, format native-1/0b14f8acfd97fa2c750e42795249dcc981178ca4/e59da9094f0c2fdda6c627fe9eff22dec6a906f0, for 26.3)
  60047 bytes, SHA-256 bab2b93cf4c96a89d037003f665614ba9019d2d1cc18edaeedbdedf1ec2b9cf8, unsigned
```

The artifact is **60,047 B**, sha256 `bab2b93cf4c96a89d037003f665614ba9019d2d1cc18edaeedbdedf1ec2b9cf8`, 61
entries, reproduced byte for byte by a second build of the final tree. (The packer takes
`horizon-mod.toml`, `Cargo.toml`, `LICENSE*`, `README.md`, `assets/`, `data/`, `src/` and the
generated `uses.json`; this `notes.md`, `AGENTS.md`, `FINDINGS.md` and `CONTENT.md` are not
packed, which is why editing them does not change the artifact.)

### Gaps after the migration

1. **No in-game run at `0b14f8a`.** `horizon-launcher` is not installed here and no game
   binary exists at this build (the shared binaries are at `1d63e0a5`/`41f822b6`). The
   behaviour claims are the *mapping* (each retired hook to a typed method/Part at the same
   vanilla call site), the type-checked glue against the pinned SDK API crate, the package
   check, and the carried rules — not a gameplay run. The 0.1.0 run stands as the behaviour's
   evidence (`evidence/e2e-default.log`, `e2e-selftest.log`); `tools/e2e.sh` is updated for a
   launcher-built game and must be run before the port claims `ported` at this pin.
2. **The same-tick take guard is a deviation that survives.** A programmatic click
   (`ctl interact`) can deliver an insert and then an empty-hand use in the same tick; the
   port skips a take in the tick an insert happened in, as 0.1.0 did. No player can click
   twice in one tick (vanilla's 4-tick use delay).
3. **No logger.** The SDK has no `log` bridge for a port; the retired `embedded_content.rs`
   was the only place that logged (`log::warn!` for an unwritable cache) and it has no
   successor, because the package no longer writes a cache file.
4. **`theurgy recipe <item>` now carries an item, not a word.** The argument is the SDK's
   typed `item` argument, so a component-bearing stack resolves to its item id; the answer
   text is unchanged.
5. **A right click no longer reaches the port from other blocks.** `right_click_block` heard
   every right click in the level; the Part hears only its own block's states, which is the
   correct semantics (and drops nothing the slice used).
6. **A block-entity tick's chunk is dirtied every tick** (the reply stays `Changed`, as the
   payload is always written), including an idle oven/brazier. Vanilla furnaces behave the
   same way while lit; an idle machine re-saving each tick is a recorded cost, not a
   behaviour change.
7. **Block entity type ids are `theurgy:calcination_oven` / `theurgy:pyromantic_brazier`.**
   The retired hook matched the *block* id; the Part is attached to the declared type, and the
   readout compares the type id. A save from 0.1.0 has the same block ids and a fresh (empty)
   compound either way.
8. **`/theurgy info` on a *vanilla* block entity names its type.** 0.1.0 printed
   `[..] is not a mod block entity.`; the SDK readout prints
   `[..] is a minecraft:chest, not a Theurgy apparatus.` The two lines the e2e parses (the
   oven's and the brazier's) are byte-identical, and `tools/parse_info.py` answers `ok=0` for
   both, so the harness is unaffected.

### Traps this migration cost

1. **The packer chooses the root documents.** `README.md` and `LICENSE*` are packed,
   `notes.md`/`AGENTS.md`/`FINDINGS.md`/`CONTENT.md`/`tools/` are not — so a note written
   after the build does not change the artifact, but a `README.md` edit does, and the
   recorded sha must be the one of the final tree. Rebuild after every packed change (the
   README rewrite moved it 59,769 → 59,865 B; the empty-hand guard moved it on to 60,047 B)
   and build twice to prove determinism.
2. **`Ok(())` inside `host.transaction` does not infer the error type** (`E0282`, “type
   annotations needed for `Result<(), _>`”). Write `Ok::<(), ()>(())` in the two closures
   whose only failure is an explicit `Err(())`.
3. **Clippy wants `?` over `let Some(x) = y else { return None }`** in a function returning
   `Option` (`clippy::question_mark`, `-D warnings`), even inside an `if` whose other arm
   returns: `let stored = stored?;`.
4. **`BlockEntityPart` is a supertrait pair and needs a declared type.** Both `impl BlockPart`
   and `impl BlockEntityPart` on the same type, and `r.block_entity_type("<path>").blocks(..).build()`
   *before* `r.block_entity_part(..)` — which returns `Result` (`unused_must_use` otherwise).
5. **There is no `item_in_tag`.** The replacement for `native_server_item_in_tag` is
   `host.tags().key(&Identifier::parse(tag))` plus `Tags::is_in_tag(item_key, tag_key)`;
   `Identifier::parse` accepts `c:gems` and defaults the namespace.
6. **`ItemStack` has getters and setters, not a public `count` field**, and `ItemStack::new`
   is fallible: `stack.count()`, `stack.set_count(n)` (returns `Result`), `ItemStack::new(key,
   n) -> Option`, `ItemStack::of(key)`.
7. **A block-entity tick has no level.** `level_in_place(host)` (the level whose block
   entities tick) is needed for the heat read, `game_time` and the `lit` write.
8. **The property write is on the host, the read on the level.** `level.block_property(host,
   pos, key)` reads; `host.set_block_property(pos, key, value)` writes and **keeps** the block
   entity (`LevelRef::set_block` would drop it).
9. **`cargo fmt` (never `--all`) is safe for `runtime.rs` only because it is already
   formatted:** verified with sha256 before and after. `--all` would format the fetched SDK,
   a sibling path dependency.
10. **The generated `uses.json` is the honest summary of the glue**, not the hook list you
    think you wrote: it names `Hooks::blocks`/`Hooks::items`/`Hooks::command` (the collection
    hooks the Parts' traits bring in) beside the twelve hooks the port really implements.
11. **`use_without_item` is also the *fall-through* for a held item.** `BlockState.useItemOn`
    returning `TRY_WITH_EMPTY_HAND` (the default) runs `useWithoutItem` **with the item still
    in the hand**, so the empty-hand take must re-check `click.stack` or a refused item takes
    the output instead of staying. The retired pin checked `game.held()` in
    `right_click_block` for the same reason; the port keeps the guard in `take_from`.
12. **The 0.1.0 content build is the right baseline for the carry check.** Comparing
    `assets/`+`data/` against `retired-hnmod-0.1.0/content-theurgy.hmod` (not against the
    working tree you just moved) is what proves the 50 carried files are unchanged; comparing
    the same directory with itself would prove nothing.

### Changelog for the first publication

> **First publication** — there is no earlier PR for `theurgy` to supersede. This is one flat
> SDK package (`api = "0.2"`, version 0.2.0, `[native]` pinned to Horizon `0b14f8a…` and
> mixins `e59da90…`) that carries both the content (`assets/`, `data/`) and the code
> (`src/`), so the retired `.hnmod` container, the `content_root` embedded-content injection
> and the plan/injection schema are gone. What changed for `api = "0.2"`: the nine `.hnmod`
> injections are typed Parts and host calls — `BlockEntityPart` on the two declared block
> entity types for the entity, its server tick and the 20-tick heat sample; `Registrar::
> block_container` for the oven's two slots and the brazier's one; `BlockPart::use_item_on`
> for an item and `use_without_item` for the empty hand (which the old pin routed through
> `right_click_block`); `Hooks::register_commands`/`run_mod_command` for the three
> `/theurgy` readouts, now with typed `block_pos` and `item` arguments. The hand now moves
> through `ItemPlace::PlayerHand` in a transaction with the block entity's slot, the block's
> `lit` is written with `set_block_property` (which keeps the block entity), and the reads use
> `LevelRef`/`BlockEntityRef`/`Tags` handles. `src/runtime.rs` and its 17 tests are
> **byte-identical** to the retired bundle's, the 86 recipes and all 50 content files are
> carried unchanged, and the artifact is 60,047 B, sha256
> `bab2b93cf4c96a89d037003f665614ba9019d2d1cc18edaeedbdedf1ec2b9cf8`.

## Review fixes (2026-11-18)

`var/review-all/1253-theurgy.md`, findings 1–2 fixed and 3 verified; version **0.2.1**
(61,660 B, sha256 `215974404b1c2edd51cba2dc7e9447151faaa9dfb0d377482087bcc63528ea4f`).

- **Brazier converts the stored fuel into the held type (src/support.rs:352).** An insert with a
  different fuel in the slot rebuilt the stack as `ItemStack::new(held.item(), old + moved)`.
  Now only the same item *and* components merges (`runtime::fuel_merges` plus
  `same_components`); a different stack refuses the click, and the merged stack starts from the
  stored stack (its components preserved), not from the held item id. Tests:
  `only_the_same_fuel_merges_in_the_brazier`.
- **Diagnostic disclosure (src/support.rs:106, :455–471).** `/theurgy info` read an apparatus's
  input/output/fuel and counts at any loaded coordinates. It now needs an operator (permission
  ≥ 2) or a player in the same level within 8 blocks; any other caller gets a generic refusal.
  Rule: `runtime::may_inspect`, pinned by
  `a_readout_needs_an_operator_reach_or_the_same_level`.
- **Licence (mods/theurgy.toml:26) — *superseded on 2026-11-19, see the section below*.** This job
  read only `REUSE.toml`'s label and shipped the CC BY 4.0 text; the next section proves that the
  text and README at the tag both say ShareAlike, so the assets are CC-BY-SA-4.0.
- **The in-game leg found a load refusal:** `r.block_part(oven, Oven)` plus
  `r.block_entity_part(oven_type, Oven)` attached one Part twice, and the game refused the
  package (`… is attached to theurgy:calcination_oven twice`). A block-entity Part is also the
  block's Part at this pin, so the two `block_part` calls are gone; the game loads "content from
  1 mod(s)" and every e2e step passes.

Ladder: `cargo test` 25 passed; `cargo clippy --all-targets -- -D warnings` exit 0;
`cargo fmt --check` clean; `cargo horizon-mod build` twice byte-identical; `check` exit 0 and
`fits --build 0b14f8a` exit 0. In-game: `PASS` + `EXIT:0`
(`ports/_shared/evidence/theurgy-e2e/e2e-launcher.log`); `GAME=$CONTROL_GAME` → `FAIL`
(`Unknown block type 'theurgy:pyromantic_brazier'`) + `EXIT:1`.

## Licence correction (2026-11-19) — the assets are CC-BY-SA-4.0; version 0.2.2

The reviewer of #1253 was right, and the evidence is stronger than the `REUSE.toml` line alone.
At the carried tag `release/v1.21.1-1.78.0` (commit `95c01bbb`, 2026-10-06) **two upstream documents
say ShareAlike**:

| Document | Says |
|---|---|
| `LICENSES/CC-BY-4.0.txt` — the licence **text** shipped for the assets (20,137 B) | **CC-BY-SA-4.0**: line 1 `Attribution-ShareAlike 4.0 International`; "The License Elements of this Public License are Attribution and ShareAlike"; Section 3(b) `ShareAlike.` |
| `README.md` §Licensing, line 61 | **CC-BY-SA-4.0**: "Assets are licensed under the CC-BY-SA-4.0 license, view [LICENSES/CC-BY-SA-4.0](./LICENSES/CC-BY-4.0.txt)." |

Only the **labels** — `REUSE.toml`'s `SPDX-License-Identifier = "CC-BY-4.0"` and the jar's
`mods.toml` string — say CC-BY-4.0. A label names a licence; the text it names supplies the terms,
and here the text and the author's own README both say ShareAlike. **The assets are CC-BY-SA-4.0;
the code stays MIT.** The counter-check that this is not a stray word: the genuine CC BY 4.0 text
(18,657 B, the file 0.2.1 shipped) has **0** "ShareAlike" occurrences and no Section 3(b), while the
tag's file has **5** (lines 1, 57, 62, 111, 273) and the whole Section 3(b) obligation.

**Upstream changed it after the release.** Commit `43a2d99cde46` on `version/1.21.1`
(2026-11-16, "chore: fix licensing info inconsistency") replaced the ShareAlike text with the CC BY
4.0 text (+102 −373) and changed the README line to match. It postdates 1.78.0 by six weeks, so it
does not rewrite what the release shipped; and because ShareAlike *adds* an obligation rather than
removing one, complying with it cannot breach the CC-BY-4.0 label either. A later slice re-cut from a
jar built after 2026-11-16 can declare `CC-BY-4.0` if its own artefact says so — recorded here so
nobody re-derives it. Full evidence (URLs, statuses, quoted clauses, commit diffs):
`var/licence-theurgy.md` in the campaign root.

### What changed in the package

| File | Change |
|---|---|
| `LICENSE-CC-BY-4.0` (18,657 B, the genuine BY text) | **removed** — it is not the text upstream ships for this release |
| `LICENSE-CC-BY-SA-4.0` | **added**, the tag's `LICENSES/CC-BY-4.0.txt` bytes verbatim (20,137 B, SHA-256 `5e436ff8…`), under the licence's real name |
| `LICENSE-NOTICE.md` | rewritten: the allocation table, the two-documents evidence, the share-alike obligation, and the attribution (`© 2024 klikli-dev`; the brazier textures additionally `© 2026 quequn07-sys`, per upstream's later per-file `REUSE.toml`) |
| `Cargo.toml` | `license = "MIT"` — one SPDX expression, the crate's own code; the asset licence is documented in the notice. `version = "0.2.2"` |
| `horizon-mod.toml` | the licence comment states code MIT / assets CC-BY-SA-4.0 and which document governs; `version = "0.2.2"` |
| `README.md`, `FINDINGS.md`, `CONTENT.md` | the same split, so no port document still claims `MIT AND CC-BY-4.0` |

The registry entry should read **`license = "MIT AND CC-BY-SA-4.0"`** (the entry is the
orchestrator's to edit). `MOD-POLICY.md` requires the entry's licence to be the one in the source
repository, and that is what the repository now says.

### A licence change must not change behaviour — it did not

`src/` is byte-identical before and after this job:
`src/lib.rs` `610c9acc…`, `src/runtime.rs` `0d88a37d…`, `src/support.rs` `555ddb45…`
(the same hashes as the 0.2.1 tree). Only licence text, comments, `README.md` and the two version
fields moved; the packed `src/` entries are unchanged, so the package's behaviour is the 0.2.1
behaviour.

### Verification (0.2.2)

| Step | Result | Evidence |
|---|---|---|
| `cargo test --offline` | **25 passed**, 0 failed | `ports/_shared/evidence/theurgy/sdk-test.log` |
| `cargo clippy --all-targets --offline -- -D warnings` | exit 0, 0 warnings | `…/sdk-clippy.log` |
| `cargo fmt --check` | clean | `…/sdk-fmt.log` |
| `cargo horizon-mod build`, twice | **63,679 B**, SHA-256 `5fe7edbdcb7cc45065727761656a261d646282c8155c8587d3a5d94d588cbc7a`, `cmp` identical | `…/sdk-build.log`, `…/sdk-build-second.log` |
| `cargo horizon-mod check theurgy.hmod` | `theurgy 0.2.2 (native-mod, format native-1/0b14f8ac…/e59da909…, for 26.3)`, 63,679 B, exit 0 | `…/sdk-check.log` |
| `cargo horizon-mod fits theurgy.hmod --build 0b14f8a` | `0b14f8a  made for`, exit 0 | `…/sdk-fits.log` |
| `python3 ports/tools/verify-sdk-package.py theurgy` | **1/1**, every check ok (runtime byte-identical `0d88a37d…`, 61 entries, no packed `Cargo.lock`) | run in session |
| in-game leg | **PASS** + `EXIT:0`, control leg inside the same run ("without the package: no oven item, the block stayed minecraft:air") | `ports/_shared/evidence/theurgy-e2e/e2e-launcher.log` |
| control (the same Nightly, no package) | **FAIL** + `EXIT:1` (`Unknown item 'theurgy:calcination_oven'`, `Unknown block type 'theurgy:pyromantic_brazier'`) | `…/e2e-launcher-control.log` |
| `python3 ports/tools/check-e2e-logs.py theurgy-e2e --strict` | `logs=2 ok=2 missing-exit=0 bad-exit=0`, exit 0 | run in session |

The packed licence files are the four `LICENSE*` entries (`LICENSE`, `LICENSE-MIT`,
`LICENSE-CC-BY-SA-4.0`, `LICENSE-NOTICE.md`); the package grew from 61,660 B to 63,679 B because the
ShareAlike text is 20,137 B where the BY text was 18,657 B, plus the longer notice.

Skills: superpowers/using-superpowers,systematic-debugging,verification-before-completion; BeautifulRust/rust-best-practices

## 26.3 dialect sweep (2026-11-21) — carried recipes and loot tables, proven in game (0.2.3)

The shared checker `ports/tools/check_261_dialect.py` (generalised from `ports/modularrouters/tools/data_shape.py`
and `ports/createaddition/tools/data_shape.py`) found the 1.21.1 forms still in the package that had shipped
at **0.2.2**; `cargo horizon-mod check`, `fits` and every unit test had passed on them, and the content
was dead in game (a recipe returns nothing; a block drops empty).

| Family | Files | 1.21.1 form that shipped | 26.3 form |
|---|---|---|---|
| recipe ingredients | 2 (`calcination_oven`, `pyromantic_brazier`): `{"tag": …}` (`c:storage_blocks/copper`, `c:ingots/iron`, `c:stones`) in `key` | `{"item": "…"}` / `{"tag": "…"}` | `"minecraft:…"` / `"#c:…"` |
| loot pools | 2 (`calcination_oven`, `pyromantic_brazier`): already singular-conditioned, but with `bonus_rolls` and `"rolls": 1.0` | `"conditions": [ … ]`, `bonus_rolls`, `"rolls": 1.0` | one `"condition": {"type": "minecraft:…"}`, no `bonus_rolls`, `"rolls": 1` |

Both loot tables dropped correctly before the sweep (their condition was singular); only the pool fields were normalised, and the loot phase reports n/a because neither table was in the array form.

**Red, kept:** `ports/_shared/evidence/theurgy/dialect-check-red.log` — the checker exits 1 on the pre-fix
package itself (`dialect-prefix-0.2.2.hmod`, 0.2.2, kept in the same folder), naming every file.
A reconstructed fixture carrying all three loot shapes plus a `requires_tool` block with no mineable tag is
red too (`../../morered/dialect-check-fixture-red.log`). The checker's `--selftest` fails when any shape is
accepted.

  `recipe calcination_oven.json: crafting_shaped -> {"count": 1, "item": "theurgy:calcination_oven", "slot": 200}`; `the result reached the inventory (1 theurgy:calcination_oven), the grid emptied, and minecraft:copper_block was consumed`

  `n/a: this port's loot tables were already in the 26.3 form`

The in-game leg is `tools/e2e-dialect.sh` (`ports/_shared/evidence/theurgy/e2e-dialect.log`): `PASS` + `EXIT:0`,
and `SELFTEST=1` inverts the expectation and ends `FAIL` + `EXIT:1` (`e2e-dialect-selftest.log`).
`check-e2e-logs.py theurgy --strict` exits 0.

**Ladder** (all in `build-slot.sh` leases, `CARGO_INCREMENTAL=0`, `BUILD_LOCAL=1`):

| Step | Command | Result |
|---|---|---|
| tests | `cargo test --offline` | **25 passed**, 0 failed (`dialect-test.log`) |
| package | `rm -f Cargo.lock && cargo horizon-mod build` (twice) | built against **0a875da** (re-pinned from 0b14f8a), second build `cmp`-identical (`dialect-build-1.log`, `dialect-build-2.log`) |
| registry check | `cargo horizon-mod check theurgy.hmod` | `theurgy 0.2.3 (native-mod, format native-1/0a875da…/e59da90…, for 26.3)`, **63,629 B**, sha256 `9e2d1543215a…` (`dialect-check.log`) |
| fits the pin | `cargo horizon-mod fits theurgy.hmod --build 0a875da` | `0a875da  made for`, exit 0 (`dialect-fits-0a875da.log`) |
| fits the previous build | `… --build 0b14f8a` | `does not fit`, exit 1 (`dialect-fits-0b14f8a.log`) |
| the gate | `python3 ports/tools/verify-sdk-package.py theurgy` | **1/1** (`dialect-gate.log`) |

Package 0.2.3: **63,629 B**, SHA-256 `9e2d1543215a32116a2ecc7f435da10390498f2c986657ce030ddeb849f33805`.

Skills: superpowers/using-superpowers,systematic-debugging,test-driven-development,verification-before-completion; BeautifulRust/none (no Rust change)
