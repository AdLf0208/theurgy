# Theurgy — a Horizon port

**Theurgy** is a Minecraft mod. An open-source magic mod built around classical alchemy to replicate and transmutate matter, and create magical artifacts. This repository is a **port of it to [Horizon](https://github.com/horizon-rust/horizon)**, the Rust engine for Minecraft Java 26.3. It carries the complete source of the port plus the content files it ships, and installs as a single `.hmod` package.

Upstream: [klikli-dev/theurgy](https://github.com/klikli-dev/theurgy) — licence **MIT**.

| | |
|---|---|
| Upstream mod | Theurgy ([source](https://github.com/klikli-dev/theurgy)) |
| Ported to | Horizon 0b14f8a, Minecraft Java 26.3, mod API `0.2` |
| Package | `theurgy.hmod` (install it with the Horizon launcher) |
| Licence | **MIT** — the upstream mod's licence; see [Licence and credit](#licence-and-credit) |

## What this port implements

### What the slice does

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

## Not yet ported

- **The other nine processes**: distillation (mercury shards, 45 recipes),
- **The other items**: 287 alchemical sulfurs, 19 niters, mercury shard/crystal,
- **The apparatus not carried**: distiller, liquefaction cauldron, fermentation
- **Heat beyond the brazier**: the Caloric Flux Emitter (remote heat over 8
- **Client half**: GeckoLib renderers (the oven's animated body, the lit rings),
- **Advancements** (9) and the two block tags the jar declares only for its ores.
- **Modded tag members**: only vanilla members exist in 26.3, so a tag ingredient
- **Hoppers pulling from the oven's input** are allowed (upstream's combined

## Installing

1. Install the [Horizon launcher](https://github.com/horizon-rust/horizon/releases) (0.2.70 or newer).
2. Add `theurgy.hmod` to the instance's mods.
3. The package is pinned to a specific Horizon build; the launcher checks that pin.

## Licence and credit

Theurgy was created by its upstream authors; the code, models and assets in the original mod
remain theirs and are used here under **MIT**. The Rust in this repository is a clean-room
rewrite for Horizon, not a translation of the original source.

- `LICENSE` — the upstream licence text.
- `LICENSE-NOTICE.md` — what this port carries and where it came from.
- The upstream project: https://github.com/klikli-dev/theurgy

## Technical details

```
horizon-mod.toml   package manifest (id, version, api, [native] pin)
Cargo.toml         the crate; depends on `horizon-mod-api` from the fetched SDK
src/               lib.rs (the one `impl Hooks`), runtime.rs (pure logic + tests), glue
assets/ data/      the content the package ships
tools/             content generators and the e2e scenario
```

- Build: `cargo horizon-mod build` (packages; it does not compile against the game).
- Test: `cargo test`, `cargo clippy --all-targets -- -D warnings`.
- In-game: `ports/tools/launcher-game.sh run theurgy` in the campaign tree, or install the
  `.hmod` and play; the harness scenario is `tools/e2e.sh`.

This repository is generated from the campaign's port at `ports/theurgy/`; the port's own
`notes.md` is the full record (decisions, traps, evidence).
