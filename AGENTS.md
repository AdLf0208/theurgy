# Working on this Horizon mod (SDK layout, mod API 0.2)

This is one flat `.hmod` SDK package: `horizon-mod.toml` is the only manifest and
identity (`id = "theurgy"`, `version = "0.3.0"`, `api = "0.2"`, `[native]` pins).
`Cargo.toml` depends only on `horizon-mod-api` from `.horizon/sdk/` (gitignored,
never packed); `mod_api::serde_json` is re-exported directly by `horizon-mod-api`.

- Keep pure logic free of `mod_api` host calls where practical: `src/engine/` holds
  the core state machines and algorithms, and each subsystem module carries unit
  tests for its pure helpers.
- `src/lib.rs` is the single `#[mod_api::horizon_mod] impl Hooks`; `src/registry.rs`
  registers the shared `TheurgyBlockPart` (`BlockPart` + `BlockEntityPart`) once per
  declared block entity type, and `src/items.rs` provides `TheurgyItemPart`.
- Never attach a `BlockPart` separately to a block that already has a `BlockEntityPart`
  (load fails with "attached twice").
- Never place extra `.toml` files (e.g. `REUSE.toml`) at the package root or non-`.rs`
  data files inside `src/` (`horizon-mod-package` refuses unexpected root `.toml` files
  and only packs `.rs`/`.toml` under `src/`; bundle data under `assets/` or `data/`).
- Prefer SDK hooks, Parts, capabilities and data declarations over `[[native.mixin]]`
  or `[[native.module]]`. Do not recreate legacy `.hnmod`, `mixins/`, `native/`, or
  `content/` layouts.

Validate with `RUSTFLAGS="-D warnings" cargo test --all-targets --offline` (plus
`cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` when those
components are installed — never `cargo fmt --all`, which formats `.horizon/sdk`),
then `rm -f Cargo.lock && cargo horizon-mod build`, `check`, and `fits`. Keep
validation claims honest: separate unit-tested/compiled status from in-game runs.
