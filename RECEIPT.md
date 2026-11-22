# Package receipt — theurgy 0.3.0

| field | value |
|---|---|
| version | `0.3.0` |
| file | `theurgy.hmod` |
| bytes | 5,611,100 |
| sha256 | `e452bbd5ca7bb16adf60a1de6a6701d3c1345759bf4fc901a72da3acca311a55` |
| source | branch `full-port` — reproducible (`rm -f Cargo.lock && cargo horizon-mod build` twice, `cmp` identical) |
| game | 26.3 |
| horizon | `0a875da91dbb0fdcb7ba8c42c11341aa229af51a` |
| horizon-mixins | `e59da9094f0c2fdda6c627fe9eff22dec6a906f0` |

## Verification of these exact bytes

- `RUSTFLAGS="-D warnings" cargo test --all-targets --offline`: **112 passed**, 0 failed, 0 warnings.
- `cargo horizon-mod check ./theurgy.hmod`: exit 0.
- `cargo horizon-mod fits ./theurgy.hmod --build 0a875da`: `0a875da  made for`, exit 0.
- **In-game verification on Horizon `0a875da` (integrated server + Vulkan GPU GUI via `horizon-ctl`)**: **34/34 scenarios + `/theurgy` commands PASS** (`all`, `interactions`, `fluids`, `sal-fluid`, `vat-interactions`, `reformation`, `flux`, `incubation-chaos`, `items`, `tools`, `divination`, `wand`, `logistics`, `logistics-advanced`, `wire-graph`, `filters`, `filter-matching`, `filter-ui`, `standard-attributes`, `special-attributes`, `attribute-filters`, `content-rules`, `worldgen`, `config-attunement`, `persistence`, `review-fixes`, `recipes`, `tooltips`, `client`, `client-features`, `book`, `item-icons`, `wire-render`, `render`).
- Not run: `cargo clippy` / `cargo fmt --check` (not bundled in the launcher Rust 1.99.1 toolchain) and Java-server `differential-*.mjs` suites (no live NeoForge Java server running). See `notes.md`.
