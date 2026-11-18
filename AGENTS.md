# Working on this Horizon mod (SDK layout, mod API 0.2)

> **Build slots (v2):** every build/test goes through the campaign runner
> (`ports/tools/build-slot.sh` from the campaign root, `../tools/build-slot.sh`
> from this port). It **blocks in the kernel** until a slot is free, so
> **never poll**: no `while ! slot; sleep` loop, no background build. Read
> `../_shared/build-slots-usage.md` before running anything. Never pass
> `--target-dir`, never run a bare `cargo` outside a lease.

This is one flat `.hmod` package: `horizon-mod.toml` is the only manifest and
the only identity (`id`, `version`, `api = "0.2"`, `[native]` with the exact
game/Horizon/mixins commits). `Cargo.toml` depends only on `horizon-mod-api`
from the fetched `.horizon/sdk/` (gitignored, never packed).

- `src/runtime.rs` is the port's **pure** logic and **must stay byte-identical**
  to the retired bundle's file (`mixins/theurgy/runtime.rs` in
  `../_shared/evidence/theurgy/retired-hnmod-0.1.0/`). Its 17 tests run with
  `cargo test` alone. Do not add `mod_api` types or tests to it.
- `src/support.rs` is the glue: the block-entity ticks, the clicks, the commands.
  Keep it free of business rules; anything worth a test belongs in `runtime.rs`
  or as a pure helper with a test in `support.rs`.
- `src/lib.rs` is the one `#[mod_api::horizon_mod] impl Hooks` plus the two
  `BlockPart`/`BlockEntityPart` Parts.
- `assets/` and `data/` are packed from the package root; the old `content/`
  folder no longer exists. Regenerate with `tools/assemble_content.py` (and
  `--check` to prove no drift).
- The retired paths (`mixins/`, `compatibility.json`, `rust-toolchain.toml`,
  `tools/horizon.py`, `tools/package.*`, the `.hnmod`) are historical evidence
  in `../_shared/evidence/theurgy/retired-hnmod-0.1.0/`; do not recreate them.

Validate with `cargo test`, `cargo clippy --all-targets -- -D warnings` and
`cargo fmt --check` (never `cargo fmt --all`: that formats the fetched SDK),
then `cargo horizon-mod build` / `check` / `fits`. Keep validation claims and
the pinned revisions honest, and never build or test against 1.21.1.
