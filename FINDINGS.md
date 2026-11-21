# Theurgy 1.78.0 — jar inventory and behaviour findings

Read from the ATM10 jar and the upstream repository, for behaviour and data
only. The Rust is a clean-room rewrite; no Java is copied and no class is
decompiled.

## Provenance

| What | Value |
|---|---|
| Jar (kept as evidence) | `ports/_shared/licence-cache/theurgy-1.21.1-neoforge-1.78.0.jar`, 5,323,672 B |
| Jar SHA-256 | `2165c80889e24f30d3351941078247c37de238c59354eaf6bc1bb278af2bc7dd` |
| Jar SHA-1 | `3e8ff57bae6610f4310ea4dfd6654b7cd8e891ca` |
| Mod id / version | `theurgy` / `1.78.0` (Minecraft 1.21.1, NeoForge `[21.0.110-beta,)`) |
| Licence | **Code MIT, carried assets CC-BY-SA-4.0** (rank 0 / rank 1). The jar's `mods.toml` and upstream's `REUSE.toml` say `CC-BY-4.0`, but the licence text shipped at the tag (`LICENSES/CC-BY-4.0.txt`) is the *Attribution-ShareAlike 4.0 International* legal code and upstream's README at that tag says "Assets are licensed under the CC-BY-SA-4.0 license" — the text governs. The port keeps `LICENSE-MIT` and ships the upstream text as `LICENSE-CC-BY-SA-4.0`; see `LICENSE-NOTICE.md` and `var/licence-theurgy.md`. Upstream changed the stated asset licence to CC-BY-4.0 on 2026-11-16 (`43a2d99c`), after this release. |
| Upstream repo | <https://github.com/klikli-dev/theurgy>, tag `release/v1.21.1-1.78.0`, commit `95c01bbbf9be19b21feea8b1c471e98bccea2a42` (shallow clone, read at `/tmp/opencode/theurgy-121`, deleted after this file) |
| Required dependencies | `neoforge`, `modonomicon`, `geckolib` (required), `almostunified`/`jei`/`jade` (optional) |

Jar contents: **3,668 files** — 637 classes, 2,790 data/asset files and the
manifest; **2,744** `en_us` lang keys.

## Families the jar actually contains (not inferred from the name)

Theurgy is a classical-alchemy mod built on the salt/mercury/sulfur triad. It
ships four atoms of content, not one:

| Family | Count in the jar | What it is |
|---|---:|---|
| Alchemical **salts** | 5 items | `alchemical_salt` (generic) + `_creature`, `_mineral`, `_plant`, `_strata` |
| Alchemical **sulfurs** | 287 items | one per material (`alchemical_sulfur_iron`, `…_apple`, `…_oak_log`, …), tagged `theurgy:alchemical_sulfurs` |
| Alchemical **niters** | 19 items | four tiers (abundant/common/rare/precious) × five kinds |
| **Mercury** | 3 items + 2 fluids | `mercury_shard`, `mercury_crystal`, `purified_gold`; `theurgy:mercury`/`sal_ammoniac` fluids |
| **Apparatus** (25 blockstates) | 25 blocks | `calcination_oven`, `pyromantic_brazier`, `distiller`, `liquefaction_cauldron`, `fermentation_vat`, `digestion_vat`, `incubator` (+3 vessels), `sal_ammoniac_accumulator`, `sal_ammoniac_tank`, `mercury_catalyst`, `caloric_flux_emitter`, `sulfuric_flux_emitter`, three reformation pedestals, six logistics nodes, `copper_wire`, two sal-ammoniac ores, two crystallized fluids |
| **Items** | 393 item models | the above plus jars, divination rods (4 tiers × 4 attunements), filters, the Hermetica book icon |
| **Recipe types** | 1,777 recipes, 10 custom types | `calcination` 86 · `distillation` 45 · `liquefaction` 358 · `fermentation` 286 · `incubation` 287 · `digestion` 31 · `reformation` 631 · `accumulation` 4 · `catalysation` 2 · crafting 47 |
| Data | 25 loot tables, 9 advancements, 80 tag files, 100 Modonomicon book pages | |
| Client | GeckoLib animated block/item renderers, 4 GUI textures, JEI/EMI/Jade plugins | |

### The ten processes, and which is the smallest complete one

| Process | Input → output | Machine |
|---|---|---|
| **Calcination** | item → 1–64 **salt** (type by material) | Calcination Oven (on heat) |
| Distillation | item (×1–10) → 1 **mercury shard** | Distiller (on heat) |
| Liquefaction | item + 10 mB sal ammoniac → 1 **sulfur** | Liquefaction Cauldron |
| Accumulation | water + crystal → sal ammoniac fluid | Accumulator + Tank |
| Catalysation | mercury crystal/shard → mercury flux | Mercury Catalyst |
| Fermentation | item + starter → **niter** | Fermentation Vat |
| Digestion | niter → niter (tier/kind change) | Digestion Vat |
| Reformation | 4 niters → 1 niter | 3 Pedestals |
| Incubation | **1 sulfur + 1 salt + 1 mercury → 1 item** | Incubator |
| Divination | — | Rods (ores) |

**Calcination is the smallest complete process**: one block on a heat source,
one input slot, one output slot, one progress counter, and 86 recipes that need
no fluid, no second block entity and no item component. It is also the first
step the mod's own book teaches ("create_salt") and it produces the salts that
incubation consumes, so the slice sits on the chain's main trunk. Distillation
is the same shape but its machine is two blocks tall as well and its output
(mercury) is consumed only at the far end of the chain.

## Calcination: the exact rules (numbers as the code and the JSONs state them)

- `CalcinationRecipe.DEFAULT_TIME = 100` ticks; **all 86 jar recipes also say
  `time: 100`** (`data/theurgy/recipe/calcination/*.json`).
- A recipe is `SizedIngredient` + result + time. `matches` tests the input slot
  with `SizedIngredient.test`, i.e. **the item must be an ingredient member and
  the stack must hold at least `count`**.
- `CraftingBehaviour.tickServer(heated, hasInput)`:
  - no input → `stopProcessing` (progress 0, `isProcessing=false`);
  - recipe found and the **whole** result fits the output slot (`insertItemStacked(…, true)` leaves nothing)
    and heated → `progress++`, and at `progress >= totalTime`: reset progress,
    insert the result, **extract `ingredientCount` from input slot 0**;
  - otherwise (`not heated`, or no recipe, or the output is full)
    → `stopProcessing`, progress back to **0** (progress never persists across a
    cold tick).
- Heat: `HeatConsumerBehaviour` samples the heat provider **below** the block
  once every **20 game ticks** (`gameTime % 20 == 0`) and caches the answer; the
  brazier's answer is its `lit` block state. Changing heat also flips the
  machine's own `lit` state.
- The machine is **two blocks tall** (`half=lower|upper`), the block entity sits
  in the lower half, and `onRemove` spills both slots.

### The input/output slots

- Input `ItemStackHandler(1)`, slot limit 64; its `isItemValid` is
  `craftingBehaviour.canProcess(stack)` (an ingredient, or the same item already
  in the slot).
- Output `ItemStackHandler(1)` wrapped in `PreventInsertWrapper`, so the world
  may only **take** from it; the machine itself inserts into it.
- Combined wrapper for capabilities: input = slot 0, output = slot 1.

### The interaction (`TwoSlotItemHandlerBehaviour.useItemOn`)

- Empty hand: take from the **output** first, then from the **input** (the
  taken stack goes to the player's inventory).
- Item in hand: insert into the **input**; the remainder stays in the hand.
- Anything else passes through to the block's default interaction.

## Pyromantic Brazier: the heat source, exactly

- One slot; `isItemValid` accepts an item whose burn time for the
  `theurgy:pyromantic_brazier` recipe type is > 0.
- Tick: if lit, `remainingLitTime -= 1`; then if **not** lit and the slot holds
  fuel, `remainingLitTime = burnTime(fuel)` and one item is consumed (a
  crafting remainder such as a lava bucket's bucket replaces the stack).
  `lit` is `remainingLitTime > 0`, written to the block state with `setBlock`
  (flags 3) when it changes.
- So one fuel item heats for exactly its burn time in ticks: **coal/charcoal
  1,600 ticks = 16 calcination crafts**, one stick 100 ticks = 1 craft.
- Blocks: `strength(1.0f)` → hardness 1.0, blast resistance 1.0; `SoundType.COPPER`;
  `noOcclusion()`; map colour METAL. The brazier is lit-light **14**; the oven
  emits no light.

## The slice's data (all read from `data/theurgy/recipe/calcination/`)

86 recipes, 70 with an item ingredient and 16 with a tag or a nested tag. All
time 100. The output counts are the interesting part:

| Input (count) | Output | Count |
|---|---|---:|
| `minecraft:blue_ice` (1) | `theurgy:alchemical_salt_strata` | **64** |
| `minecraft:packed_ice` (1) | `…_strata` | 9 |
| `theurgy:crystallized_lava` (1) | `…_strata` | 8 |
| `minecraft:clay` (1) | `…_strata` | 4 |
| `theurgy:crystallized_water` (1) | `…_strata` | 4 |
| `minecraft:crying_obsidian` (1) | `…_strata` | 3 |
| `minecraft:obsidian`, `…_end_stone`, `…_magma_block`, `…_terracotta`, `…_nylium` (1) | `…_strata` | 2 |
| `minecraft:blackstone`, `…_gravel`, `…_stone`, `…_snowball`, … (1) | `…_strata` | 1 |
| `theurgy:alchemical_salt_strata` (5) | `theurgy:alchemical_salt_mineral` | 1 |
| `#c:gems` (1) | `…_mineral` | 2 |
| `#c:ores` (1) | `…_mineral` | 3 |
| `#c:ingots` (1) | `…_mineral` | 2 |
| `#c:raw_materials` (1) | `…_mineral` | 1 |
| `#c:crops`, `#minecraft:leaves`, `#minecraft:logs`, `#minecraft:saplings` (1) | `…_plant` | 1 |
| `#c:stones`, `#c:cobblestones`, `#minecraft:sand`, `#minecraft:dirt`, … (1) | `…_strata` | 1 |
| creature items (1) — meat, bone, arrow, egg, … | `…_creature` | 1 |
| `minecraft:dragon_egg`, `…_nether_star`, `…_heart_of_the_sea`, `…_elytra`, `…_ender_pearl`, `…_ghast_tear`, `…_shulker_shell`, `…_wither_skeleton_skull`, `…_prismarine_*`, `…_rabbit_foot` (1) | `…_creature` | 3–5 |
| `theurgy:alchemical_salt_plant` (**2**) | `…_creature` | 1 |

The full generated table the port ships is `tools/recipes.json` (written by
`tools/gen_recipes.py` from the jar; `--check` re-reads the jar and fails if the
table in `runtime.rs` drifted). Tag members are resolved with
`tools/tag_members.json`, generated as follows:

- `minecraft:*` tags → the prepared **26.3** tag files
  (`/home/adlf/Desktop/horizon/.local/horizon-client/26.3/data/minecraft/tags/item/…`),
- `c:*` and `theurgy:*` tags → NeoForge's generated `c:` tags for 1.21.x
  (`https://raw.githubusercontent.com/neoforged/NeoForge/1.21.x/src/generated/resources/data/c/tags/item/…`)
  followed recursively into `c:<tag>/<material>`, plus the jar's own
  `data/theurgy/tags/item/*.json`.

Only **vanilla** members exist in 26.3; a tag's modded members (another mod's
ingot, another mod's ore) are recorded as a gap in `notes.md`.

## Where the rewrite deliberately differs (detail in notes.md)

1. **One block, not two.** The oven and the brazier are `half=lower|upper` in
   upstream. The port ships each as a single block; the model is carried for the
   brazier and is a vanilla placeholder for the oven (upstream draws the oven
   with a GeckoLib block-entity renderer, so the jar's block model is empty).
2. **No item-tag reads.** The pin exposes no item-tag membership helper, so tag
   ingredients are resolved to explicit member lists at generation time.
3. **No Caloric Flux Emitter.** Heat comes from a lit brazier directly below, as
   upstream's primary path; the emitter's remote heat is a gap.
4. `light` is a constant in the content schema, so the brazier's lit-light 14
   cannot follow its state; it is declared 0 (as unlit) and recorded.
