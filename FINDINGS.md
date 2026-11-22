# Provenance & Architecture Findings — Theurgy 0.3.0

## Upstream source

- **Original mod**: `klikli-dev/theurgy`, tag `release/v26.3-1.120.1` (commit `33b959c9ed0da1de2aff649e9728c1a03470b1bd`), Minecraft Java 26.3 NeoForge.
- **Port source repository**: `ReenkyPig-Studio/theurgy-horizon` (branch `horizon/26.3`, originally built against Horizon `fdc87ae…` in the legacy `.hnmod` / `horizon-mixin.toml` format).
- **Target SDK package**: `AdLf0208/theurgy`, flat SDK layout (`api = "0.2"`), pinned to Horizon `0a875da91dbb0fdcb7ba8c42c11341aa229af51a` and mixins `e59da9094f0c2fdda6c627fe9eff22dec6a906f0`.

## Key architectural upgrades in the SDK port

1. **Zero mixins or native modules**: Every former `native_*` host call, raw mixin (`sal_mesh` on `Mesher::mesh_reusing`), and injected game module (`native_items.rs`, `native_fluids.rs`, `native_block_entity_draw.rs`, `mesher/fluid.rs`) was replaced by first-class `horizon-mod-api` 0.2 primitives:
   - World fluid simulation & meshing: `data/theurgy/horizon/fluid/sal_ammoniac.json` + `Hooks::register_fluid_models`.
   - Filtered item & fluid transfers: `mod_api::ITEMS` / `mod_api::FLUIDS` + `HostTransaction` + `mod_api::move_resources`.
   - Custom client screens (Hermetica, Recipe Viewer, List/Attribute Filters): `ClientHost::mod_screen_open` / `mod_screen_close` / `mod_screen` + `DrawContext`.
   - Client↔server messages: `#[derive(PayloadCodec)]` typed channels (`theurgy:book`, `theurgy:wand_mode`, `theurgy:filter_edit`) with `PayloadWork` main-thread execution.
   - Reload-stable recipe caching: `Hooks::tags_updated` + `Host::recipes_of_type` / `recipe_ingredient` / `recipe_result` (no per-tick recipe JSON parsing).
   - Worldgen: 26.3 `worldgen/feature` + `worldgen/placed_feature` + `Registrar::declare_biome_modifier`.
2. **Zero external Cargo dependencies**: `horizon-mod-api` re-exports `serde_json` (`mod_api::serde_json`), so `Cargo.toml` keeps `horizon-mod-api` as its sole dependency (no `native-lock` changes needed).
3. **One minor visual gap**: `BlockEntityDrawContext` does not expose `END_PORTAL_LAYER`, so `logistics_nexus`'s 4px `Portal` bone renders with a dark-tinted `block/obsidian` quad instead of the End portal shader (`portal.rs`). Recorded in `notes.md` §8.
