# theurgy (Horizon port)

A Horizon port of [theurgy](klikli-dev/theurgy) — MIT.

**Status: first slices.** What is below is what the port actually does today.

## Implemented so far

- `theurgy:pyromantic_brazier`** (`minecraft:copper_block` template, declared
- `lit`): one fuel slot; `PyromanticBrazierBlockEntity.tickServer` exactly — burn
- down first (`remainingLitTime -= 1`), then, only when out, load the next item's
- burn time and consume one (a crafting remainder such as a lava bucket's bucket
- replaces it). `lit = remaining > 0`, written to the block state only when it
- changes. One coal/charcoal is **1,600 ticks = 16 calcination crafts**.
- `theurgy:calcination_oven`** (`copper_block` template, declared `lit`): input
- slot 0, output slot 1; heat is the block **below** being a lit brazier, sampled

## Not yet ported

- The other nine processes**: distillation (mercury shards, 45 recipes),
- liquefaction (sulfurs, 358, needs the sal ammoniac fluid), fermentation (286),
- digestion (31), incubation (287), reformation (631), accumulation (4),
- catalysation (2) — with their machines, fluids and menus.
- The other items**: 287 alchemical sulfurs, 19 niters, mercury shard/crystal,
- purified gold, jars, divination rods (4 tiers × 4 attunements), filters,
- ceramic jars, sal ammoniac crystal/bucket.
- The apparatus not carried**: distiller, liquefaction cauldron, fermentation

## Install

Build the bundle with a native-mod capable launcher, or add the file from the
registry once it is listed. Pinned to Horizon `1d63e0a` and horizon-mixins
`5a64af93` for game 26.3.

## Build from source

```sh
cargo test
python3 tools/horizon.py check --horizon <horizon checkout>
python3 tools/horizon.py build --horizon <horizon checkout>
python3 tools/package.py
```

## Credits and licence

Original mod: klikli-dev/theurgy. Port: AdLf0208. Licence: MIT — see [LICENSE](LICENSE);
the original notices are preserved.
