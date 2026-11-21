# Licence allocation — Theurgy for Horizon

This package is a **derivative work** of Theurgy 1.78.0 by klikli-dev. It carries two
licences because upstream uses two: **MIT for the code** and **CC-BY-SA-4.0 for the
assets**. The registry entry names the package as a whole, `MIT AND CC-BY-SA-4.0`.

| Part | Licence | File | Where it was read |
|---|---|---|---|
| The mod's **code**, and this port's clean-room Rust rewrite of its behaviour (`src/`, the manifests) | **MIT** | `LICENSE`, `LICENSE-MIT` | `LICENSES/MIT.txt` at the tag, and the `SPDX-License-Identifier: MIT` header on every upstream source file; the jar's `META-INF/neoforge.mods.toml` declares MIT |
| The mod's **assets** — the models, textures, lang and data this port carries, and this port's own additions to `assets/` and `data/` | **CC-BY-SA-4.0** | `LICENSE-CC-BY-SA-4.0` | the licence text shipped at the tag, `LICENSES/CC-BY-4.0.txt` (**Attribution-ShareAlike 4.0 International**), and the repository's `README.md` at the same tag: "Assets are licensed under the CC-BY-SA-4.0 license" |
| Six Create-derived GUI textures in the jar (not carried by this port) | MIT | — | `assets/theurgy/textures/{item/attribute_filter,item/list_filter,gui/filters,gui/player_inventory,gui/widgets,gui/icons}.png.license`, each `SPDX-FileCopyrightText: 2019 simibubi`, `SPDX-License-Identifier: MIT` |
| The Gradle wrapper (not carried) | Apache-2.0 | — | `REUSE.toml` for `gradle/**` and `gradlew**` |

## Which document governs, and why it is not the friendlier label

At the carried tag `release/v1.21.1-1.78.0` (commit `95c01bbbf9be19b21feea8b1c471e98bccea2a42`,
2026-10-06) upstream's documents disagree, two against two:

| Document | Says |
|---|---|
| `LICENSES/CC-BY-4.0.txt` (20,137 B) — the licence **text** shipped for the assets | **CC-BY-SA-4.0**: it opens `Attribution-ShareAlike 4.0 International`, states "The License Elements of this Public License are Attribution and ShareAlike", and carries Section 3(b) `ShareAlike.` |
| `README.md` §Licensing — the author's **own description** | **CC-BY-SA-4.0**: "Assets are licensed under the CC-BY-SA-4.0 license, view [LICENSES/CC-BY-SA-4.0](./LICENSES/CC-BY-4.0.txt)." |
| `REUSE.toml` — the SPDX **identifier** | `SPDX-License-Identifier = "CC-BY-4.0"` for `src/main/resources/**` and `src/generated/resources/**` |
| the jar's `META-INF/neoforge.mods.toml` — a **label** | `license = "MIT AND CC-BY-4.0"` |

A label names a licence; only the text supplies its terms, and here the text — and the
README that describes it — both say ShareAlike. **The port therefore takes the assets as
CC-BY-SA-4.0** and ships that actual text, unmodified, as `LICENSE-CC-BY-SA-4.0`
(SHA-256 `5e436ff8ffbb77d8607220e9bce20c8915d860010feeb6c1ebef5a85688e9b39`). The earlier
0.2.1 package shipped the genuine *Attribution 4.0* text instead, which is a licence
upstream did not ship for this release; that file is gone.

**Upstream changed this after the release.** Commit `43a2d99cde46` on `version/1.21.1`
(2026-11-16, "chore: fix licensing info inconsistency") replaced the ShareAlike text with
the CC BY 4.0 text and changed the README line to match. That commit postdates 1.78.0 by
six weeks, so it does not rewrite what 1.78.0 shipped; and because ShareAlike adds an
obligation rather than removing one, complying with it cannot breach the CC-BY-4.0 label
either. A later slice re-cut from a jar built after 2026-11-16 can declare `CC-BY-4.0` if
that is what its own artefact says.

## What the ShareAlike obligation means here

- **Attribution** — credit klikli-dev, link the upstream project, and say that this is an
  altered version. The carried files are listed in `CONTENT.md`; the attribution is
  reproduced in the package (`README.md`, this notice) and in the delivered archive.
- **ShareAlike** — the content half of this package (`assets/`, `data/`) is itself
  distributed under **CC-BY-SA-4.0**, including this port's own additions to it. Anyone
  who redistributes an adaptation of that content must license their adaptation under
  CC-BY-SA-4.0, or a BY-SA compatible licence, and keep this attribution.
- **The Rust code is not covered by that.** `src/` is a clean-room rewrite of behaviour,
  not an adaptation of upstream's Java, and is MIT — as is upstream's own code. The MIT
  terms do not reach the carried art, and the ShareAlike terms do not reach the Rust.

Attribution: **© 2024 klikli-dev, CC-BY-SA-4.0.** Upstream's later `REUSE.toml` (branch
`version/1.21.1`) additionally credits the block textures this port carries — including
`pyromantic_brazier.png` and `pyromantic_brazier_lit.png` — as
`["2024 klikli-dev", "2026 quequn07-sys"]`; both are credited here.

Evidence, URLs, HTTP statuses and the quoted ShareAlike clause: `var/licence-theurgy.md`
in the porting campaign. Upstream: <https://github.com/klikli-dev/theurgy>.
