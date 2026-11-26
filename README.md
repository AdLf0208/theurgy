# Theurgy — Horizon SDK port (`0.3.0`)

Full native Horizon SDK port (`api = "0.2"`, Minecraft 26.3, pinned Horizon `0a875da91dbb0fdcb7ba8c42c11341aa229af51a`) of **Theurgy 1.120.1** (`klikli-dev/theurgy`, `release/v26.3-1.120.1`).

- **Rust code (`src/`, manifests)**: MIT (`LICENSE`, `LICENSE-MIT`, `LICENSES/MIT.txt`).
- **Upstream assets & data (`assets/`, `data/`)**: CC-BY-4.0 (`LICENSE-CC-BY-4.0`, `LICENSES/CC-BY-4.0.txt`), with third-party notices in [`LICENSE-NOTICE.md`](LICENSE-NOTICE.md) and [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
- **Status**: compiles cleanly with `-D warnings` (0 warnings in both test and release builds), **119 unit tests passing**, **34/34 in-game scenarios passing** on Horizon `0a875da` (headless + Vulkan GPU GUI, including 3D render-reference silhouettes, Hermetica book, filter UI, wand, native sal ammoniac world fluid, worldgen, reformation, incubation chaos, logistics, filters, persistence, and `/theurgy` commands), and deterministic `cargo horizon-mod build` / `check` / `fits` passing. See [`notes.md`](notes.md) for the complete transfer map, bug fixes, and remaining API gap (`portal.rs`).

## What is ported

- **Five processing machines + heat**: Pyromantic Brazier, Calcination Oven, Distiller, Liquefaction Cauldron, Fermentation Vat, Digestion Vat — driven by the pure `src/engine/` core and reload-stable server recipe cache (`tags_updated`).
- **Sal Ammoniac native world fluid & apparatus**: `data/theurgy/horizon/fluid/sal_ammoniac.json` + `register_fluid_models` (still/flow/overlay sprites), Sal Ammoniac Accumulator & Tank, `fluid_util::interact_with_fluid_handler` bucket/tank clicks.
- **Incubation, Reformation, Mercury Flux**: Incubator + 3 vessels (with `theurgy:tag` output resolution), Sulfuric Flux Emitter + source/target/result pedestals, Mercury Catalyst, Mercury Capacitor (per-side modes), Mercury Flux Emitter (separate Mercury Flux & FE buffers), Caloric Flux Emitter (`hotUntil`), and emitter point selection.
- **Items & Tools**: Double-high apparatus placement/break, directional attachment survival, Divination Rods (attunement, 3D spherical scan, 2D distance bucket, durability, cooldown, sulfur-attuned crafting), Mercurial Wand (5 modes), Copper Wire, Nexus UUIDs, full `TooltipHandler` Shift/Ctrl tooltips, derivative jar item decorations, and `/theurgy` + `/theurgy_probe` + `/theurgy_filter` commands.
- **Logistics & Filters**: Logistics Nexus, Connector Node, Item/Fluid Inserters & Extractors, Mercury Flux Connector, Capability Probe & Proxy, Copper Wire graphs, List & Attribute Filters (all 14 standard rules + 4 special rules + tag rules) over transactional `mod_api::ITEMS` / `mod_api::FLUIDS` + `move_resources`.
- **Worldgen**: Data-driven 26.3 `worldgen/feature/sal_ammoniac_ore.json` + `worldgen/placed_feature/sal_ammoniac_ore.json` + `Registrar::declare_biome_modifier`.
- **Client**: GeckoLib 5 `.geo.json` + `.animation.json` block entity renderer, `DigestionVatRenderer` vanilla parts, wire slack curves, reformation trails (glowing motes), emitter selection outlines, held-stack fit outlines, tank fluid quads, the sulfur/niter jar special item model (`theurgy:alchemical_derivative`), Modonomicon Hermetica book screen + per-player persistence, read-only recipe viewer, CodeDefinedGUI-style list and attribute filter screens, in-world and item HUDs, key mappings, and typed payloads (`theurgy:book`, `theurgy:wand_mode`, `theurgy:wand_frequency`, `theurgy:filter_edit`, `theurgy:selection`, `theurgy:hud_request`, `theurgy:hud_snapshot`). Zero `[[native.mixin]]` or `[[native.module]]` entries required.

## Layout

```
horizon-mod.toml        manifest: id "theurgy", version 0.3.0, api "0.2", [native] pins
Cargo.toml              only dependency: horizon-mod-api (re-exports serde_json)
src/
  lib.rs                single #[mod_api::horizon_mod] impl Hooks
  engine/               pure logic core (crafting, recipes, machines, wire, performance)
  registry.rs           29 block entities, 30 components, containers, fluid/energy caps, biome modifier
  recipes.rs            reload-stable recipe cache (9 Theurgy types + 3 vanilla cooking types)
  host.rs, config.rs    NBT/crafting/tank/SDK helpers & optional JSON config
  heat.rs, machines.rs  brazier & 5 processing machines + vat redstone/analog signal cache
  fluids.rs, interactions.rs   sal ammoniac fluid models, accumulator/tank, apparatus clicks
  incubation.rs, reformation.rs, flux.rs, selection.rs
  items.rs, probe.rs, commands.rs
  logistics.rs, filter.rs
  render.rs, model_data.rs, book.rs, client.rs, recipe_client.rs, filter_client.rs
assets/ data/           Theurgy 1.120.1 (26.3) content + worldgen feature + world fluid declaration
theurgy.hmod            built deterministic SDK package (see RECEIPT.md)
```

## Build & validate

```sh
RUSTFLAGS="-D warnings" cargo test --all-targets --offline
rm -f Cargo.lock && cargo horizon-mod build
cargo horizon-mod check target/horizon-mod/theurgy.hmod
cargo horizon-mod fits target/horizon-mod/theurgy.hmod --build 0a875da
```
