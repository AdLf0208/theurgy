# Theurgy — Horizon port (slice 1: the calcination oven)

A native Horizon port of **Theurgy 1.78.0** (`klikli-dev/theurgy`): the **code is
MIT**, the **assets are CC-BY-SA-4.0** — the licence text shipped at the carried tag
says ShareAlike, and the port follows the text rather than the label. The Pyromantic
Brazier burns furnace fuel, and the Calcination Oven above it turns one item into its
alchemical salt — one blackstone into one Strata Salt, one packed ice into nine, one gem
into two Mineral Salts, all in the jar's own 100 ticks.

**LOCAL-ONLY** until the maintainer approves publishing (ARCHITECTURE.md §8). The code's
MIT is rank 0; the assets' share-alike is rank 1 (publishable under the same licence with
attribution). See [`LICENSE-NOTICE.md`](LICENSE-NOTICE.md).

Provenance is in [`FINDINGS.md`](FINDINGS.md), the carried and written content
in [`CONTENT.md`](CONTENT.md), and the deviations, gaps and verification in
[`notes.md`](notes.md).

## One file

`theurgy.hmod` is the deliverable: one flat SDK package (`api = "0.2"`) that
carries the **code** (`horizon-mod.toml`, `Cargo.toml`, `src/`) **and the
content** (`assets/`, `data/`) in the same archive. There is no embedded-content
injection and no `.hnmod` any more — Horizon's mod-API merge (#16941, launcher
≥ 0.2.70) retired that container, and the launcher compiles the package's Rust
against its own pinned game build.

## What the slice does

* `theurgy:pyromantic_brazier` — one fuel slot, furnace-fuel burn times, a `lit`
  state that follows the burn; one coal is 1,600 ticks = 16 crafts.
* `theurgy:calcination_oven` — one input and one output slot, a 100-tick
  progress that only advances while the block **below** it is a lit brazier
  (sampled once every 20 game ticks, as `HeatConsumerBehaviour` does), and the
  jar's **86 calcination recipes** as data: 70 by item, 16 by tag (`#c:gems`,
  `#c:ores`, `#minecraft:logs`, …), with the jar's own counts (packed ice → 9,
  blue ice → 64, gems → 2, ores → 3, plant salt ×2 → creature salt).
* Clicks: an item goes into the input (upstream's `TwoSlotItemHandlerBehaviour`),
  an empty hand takes the output and then the input, the brazier takes fuel and
  gives it back.
* `/theurgy info <pos>`, `/theurgy recipe <item>` and `/theurgy recipes` for the
  harness and for players who want the jar's numbers in text.

## Layout

```
horizon-mod.toml            the only manifest: id, version 0.2.2, api 0.2, [native] pins
Cargo.toml                  one crate; the only dependency is horizon-mod-api from .horizon/sdk/
src/lib.rs                  the one `impl Hooks` and the two block-entity Parts
src/runtime.rs              the pure engine: 86 recipes, the oven, the brazier (17 tests)
src/support.rs              game glue: block entities, heat, clicks, commands (+ 6 tests)
assets/theurgy/             carried models/textures + 26.3 blockstates, item definitions, lang
data/theurgy/               block/item declarations, recipes, loot, tags, creative tab
theurgy.hmod                the deliverable
tools/
  gen_tag_members.py        tag membership, resolved once into JSON
  gen_recipes.py            the jar's recipes → src/runtime.rs (`--check` guards drift)
  assemble_content.py       the jar's assets + this port's declarations → assets/ + data/
  e2e.sh                    the in-game scenario (default / SELFTEST / CONTROL_GAME)
  parse_info.py             `/theurgy info` → shell fields
notes.md FINDINGS.md CONTENT.md
```

## Build

Every step goes through a build slot from the campaign root, with `CARGO_INCREMENTAL=0`:

```sh
cd ports/theurgy
cargo test --offline                                  # 23 tests (17 runtime + 6 glue helpers)
cargo clippy --all-targets --offline -- -D warnings
cargo fmt --check
rm -f Cargo.lock && cargo horizon-mod build           # -> target/horizon-mod/theurgy.hmod
cp target/horizon-mod/theurgy.hmod ./theurgy.hmod
cargo horizon-mod check theurgy.hmod
cargo horizon-mod fits theurgy.hmod --build nightly
```

The in-game scenario needs a game the launcher built with this package
(`horizon-launcher native-build ports/theurgy --instance <i> --trust-native-code`)
and then `GAME=<that horizon-client> tools/e2e.sh`; no launcher exists in this
environment, so the run is a recorded gap.
