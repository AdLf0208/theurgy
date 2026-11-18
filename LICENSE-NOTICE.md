# Licence allocation — Theurgy for Horizon

This port is a **derivative work** of Theurgy 1.78.0 by klikli-dev. The mod
declares **two** licences and this port keeps both, split the way upstream's own
`REUSE.toml` splits them — the registry entry says `MIT AND CC-BY-4.0` for that
reason, and this table is the declaration the reviewer asked for.

| Part | Licence | File | Where it was read |
|---|---|---|---|
| The mod's **code**, and this port's clean-room Rust rewrite of its behaviour | **MIT** | `LICENSE`, `LICENSE-MIT` | `META-INF/neoforge.mods.toml` (`license = "MIT AND CC-BY-4.0"`), every source file's `SPDX-License-Identifier: MIT`, `LICENSES/MIT.txt` |
| The mod's **assets** — the models, textures, lang and data this port carries | **CC-BY-4.0** | `LICENSE-CC-BY-4.0` | the same `neoforge.mods.toml`, and `REUSE.toml`'s `SPDX-License-Identifier = "CC-BY-4.0"` for `src/main/resources/**`, `src/main/templates/**` and `src/generated/resources/**` |
| The Gradle wrapper (not carried) | Apache-2.0 | — | `REUSE.toml` for `gradle/**` and `gradlew**` |

Both carried licences are **rank 0** (permissive). Attribution for the assets:
© 2024 klikli-dev, CC-BY-4.0; the carried files are listed in `CONTENT.md`, and
the attribution above travels inside the delivered archive (the packager writes
`LICENSE`, `LICENSE-MIT`, `LICENSE-CC-BY-4.0` and this file into
`mixins/theurgy/`).

**A note on upstream's licence folder:** `LICENSES/CC-BY-4.0.txt` in the
upstream repository holds the *Attribution-ShareAlike* 4.0 text, although the
package, `REUSE.toml` and the jar all declare `CC-BY-4.0`. The port therefore
ships the official **Attribution 4.0 International** legal code
(`LICENSE-CC-BY-4.0`, from <https://creativecommons.org/licenses/by/4.0/legalcode.txt>),
which is the licence the mod declares, and records the mislabelled file here
rather than inheriting a share-alike term the mod never declared.
