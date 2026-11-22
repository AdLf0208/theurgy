# Package receipt — theurgy 0.3.0

| field | value |
|---|---|
| version | `0.3.0` |
| file | `theurgy.hmod` |
| bytes | 5,605,522 |
| sha256 | `d6646f09a13f670fade398cfe0c1dbc5d42d719ebf579e54bb29de83d7049764` |
| source | branch `full-port` — reproducible (`rm -f Cargo.lock && cargo horizon-mod build` twice, `cmp` identical) |
| game | 26.3 |
| horizon | `0a875da91dbb0fdcb7ba8c42c11341aa229af51a` |
| horizon-mixins | `e59da9094f0c2fdda6c627fe9eff22dec6a906f0` |

## Verification of these exact bytes

- `RUSTFLAGS="-D warnings" cargo test --all-targets --offline`: **112 passed**, 0 failed, 0 warnings.
- `cargo horizon-mod check ./theurgy.hmod`: exit 0.
- `cargo horizon-mod fits ./theurgy.hmod --build 0a875da`: `0a875da  made for`, exit 0.
- **Not verified in-game yet**: `horizon-launcher` (`native-build`) is not present in this environment; `cargo clippy` and `cargo fmt` binaries are not bundled in the launcher Rust toolchain. See `notes.md`.
