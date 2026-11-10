#!/usr/bin/env python3
"""Assemble Theurgy's content half from the jar, deterministically.

Reads the ATM10 jar (read-only, kept as evidence) and writes `content/`:

  * carried byte for byte — the brazier's two Blockbench models and their two
    textures, the brazier's item model, `alchemical_salt`'s model and texture,
    the two crafting recipes and the two loot tables (rewritten to 26.3's
    singular typed condition);
  * written by this port — the block declarations (`horizon/block/`, on the
    `minecraft:copper_block` template with a declared `lit` property), the five
    salt item declarations, the 26.3 item definitions, the blockstates, the
    lang subset, the creative tab and the `c:` item tags whose members
    `tools/tag_members.json` resolved.

The oven's model is a vanilla placeholder: upstream draws it with a GeckoLib
block-entity renderer, so its jar block model holds only a particle texture.

    python3 tools/assemble_content.py           # write content/
    python3 tools/assemble_content.py --check   # fail if content/ drifted
"""
import argparse
import json
from pathlib import Path
import shutil
import sys
import zipfile

ROOT = Path(__file__).resolve().parents[1]
JAR = ROOT.parent / "_shared/licence-cache/theurgy-1.21.1-neoforge-1.78.0.jar"
TAGS = ROOT / "tools/tag_members.json"
CONTENT = ROOT / "content"

SALTS = ["alchemical_salt", "alchemical_salt_creature", "alchemical_salt_mineral",
         "alchemical_salt_plant", "alchemical_salt_strata"]
BLOCKS = ["calcination_oven", "pyromantic_brazier"]
ITEMS = SALTS + BLOCKS

BLOCK_STATES = {
    # `lit` follows the heat (upstream's `HeatConsumerBehaviour` writes it).
    "calcination_oven": {"lit": {"type": "boolean"}},
    "pyromantic_brazier": {"lit": {"type": "boolean"}},
}
# Upstream: `Properties.of().mapColor(MapColor.METAL).noOcclusion()
# .sound(SoundType.COPPER).strength(1.0f)`, and 14 light when the brazier is lit
# (the content schema's `light` is constant, so it stays 0: recorded gap).
BLOCK_PROPERTIES = {
    "calcination_oven": {"hardness": 1.0, "explosion_resistance": 1.0, "light": 0,
                         "requires_tool": False, "sounds": "minecraft:copper_block",
                         "occludes": False, "map_color": 6},
    "pyromantic_brazier": {"hardness": 1.0, "explosion_resistance": 1.0, "light": 0,
                           "requires_tool": False, "sounds": "minecraft:copper_block",
                           "occludes": False, "map_color": 6},
}
ITEM_TEMPLATE = "minecraft:gunpowder"


def carried_files():
    """Jar paths copied byte for byte into the content mod."""
    files = [
        "assets/theurgy/models/block/pyromantic_brazier.json",
        "assets/theurgy/models/block/pyromantic_brazier_template.json",
        "assets/theurgy/models/block/pyromantic_brazier_lit.json",
        "assets/theurgy/models/block/pyromantic_brazier_lit_template.json",
        "assets/theurgy/models/item/pyromantic_brazier.json",
        "assets/theurgy/models/item/alchemical_salt.json",
        "assets/theurgy/textures/block/pyromantic_brazier.png",
        "assets/theurgy/textures/block/pyromantic_brazier_lit.png",
        "assets/theurgy/textures/item/alchemical_salt.png",
    ]
    return files


def written_json(document):
    return (json.dumps(document, indent=2, sort_keys=True) + "\n").encode()


def content_files():
    """Every file the content mod holds, by its path inside `content/`."""
    files = {}
    with zipfile.ZipFile(JAR) as jar:
        for name in carried_files():
            files[name] = jar.read(name)
        lang = json.loads(jar.read("assets/theurgy/lang/en_us.json"))
        recipes = {name: json.loads(jar.read(f"data/theurgy/recipe/crafting/shaped/{name}.json"))
                   for name in BLOCKS}
        loot = {name: json.loads(jar.read(f"data/theurgy/loot_table/blocks/{name}.json"))
                for name in BLOCKS}

    files["horizon-mod.toml"] = (
        "# Theurgy's content half: the calcination oven, the pyromantic brazier and\n"
        "# the five alchemical salts. Models and textures are upstream's\n"
        "# (CC-BY-4.0); see ../../LICENSE-CC-BY-4.0 and CONTENT.md.\n"
        'id = "theurgy"\n'
        'name = "Theurgy"\n'
        'version = "0.1.0"\n'
        'api = "0.2"\n'
        'side = "both"\n'
        'capabilities = ["registries"]\n'
    ).encode()

    for block in BLOCKS:
        files[f"data/theurgy/horizon/block/{block}.json"] = written_json({
            "template": "minecraft:copper_block",
            "state_properties": BLOCK_STATES[block],
            "default_state": {"lit": "false"},
            "properties": BLOCK_PROPERTIES[block],
        })
        # 26.3 item definitions.
        model = ("minecraft:block/furnace" if block == "calcination_oven"
                 else f"theurgy:block/{block}")
        files[f"assets/theurgy/items/{block}.json"] = written_json(
            {"model": {"type": "minecraft:model", "model": model}})
        # Loot: upstream's table with the two-block `half` condition folded away
        # and 26.3's singular typed condition.
        files[f"data/theurgy/loot_table/blocks/{block}.json"] = written_json({
            "type": "minecraft:block",
            "pools": [{
                "bonus_rolls": 0.0,
                "entries": [{"type": "minecraft:item", "name": f"theurgy:{block}"}],
                "rolls": 1.0,
                "condition": {"type": "minecraft:survives_explosion"},
            }],
            "random_sequence": f"theurgy:blocks/{block}",
        })
        # The crafting recipes as the jar writes them; their `c:` tags are
        # declared below, since 26.3 ships no convention tags.
        files[f"data/theurgy/recipe/{block}.json"] = written_json(recipes[block])

    files["assets/theurgy/blockstates/calcination_oven.json"] = written_json({
        "variants": {
            "lit=false": {"model": "minecraft:block/furnace"},
            "lit=true": {"model": "minecraft:block/furnace_on"},
        }
    })
    files["assets/theurgy/blockstates/pyromantic_brazier.json"] = written_json({
        "variants": {
            "lit=false": {"model": "theurgy:block/pyromantic_brazier"},
            "lit=true": {"model": "theurgy:block/pyromantic_brazier_lit"},
        }
    })

    for salt in SALTS:
        files[f"data/theurgy/horizon/item/{salt}.json"] = written_json({
            "template": ITEM_TEMPLATE,
        })
        files[f"assets/theurgy/items/{salt}.json"] = written_json(
            {"model": {"type": "minecraft:model", "model": "theurgy:item/alchemical_salt"}})

    names = {f"block.theurgy.{block}": lang[f"block.theurgy.{block}"] for block in BLOCKS}
    for salt in SALTS:
        # Upstream's item name is the `%s` template `item.theurgy.<salt>` with
        # the `.source` suffix; 26.3 content lang has no format arguments, so
        # the template is expanded per item (the generic salt has no upstream
        # key at all and gets the bare name).
        key = f"item.theurgy.{salt}"
        if f"{key}.source" in lang:
            names[key] = lang[key].replace("%s", lang[f"{key}.source"])
        else:
            names[key] = "Alchemical Salt"
    files["assets/theurgy/lang/en_us.json"] = written_json(names)

    files["data/theurgy/horizon/creative_tab/theurgy.json"] = written_json({
        "title": "Theurgy",
        "icon": "theurgy:calcination_oven",
        "items": [f"theurgy:{item}" for item in ITEMS],
    })

    # The `c:` item tags the recipes name, with the members resolved once in
    # tools/tag_members.json (26.3 vanilla tags, NeoForge's generated c: tags
    # and the jar's own tags).
    tags = json.loads(TAGS.read_text())
    for tag, entry in tags.items():
        if not entry["known"]:
            continue
        namespace, path = tag.split(":", 1)
        files[f"data/{namespace}/tags/item/{path}.json"] = written_json(
            {"values": entry["members"]})
    return files


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    files = content_files()
    if args.check:
        stale = []
        for name, data in files.items():
            path = CONTENT / name
            if not path.is_file() or path.read_bytes() != data:
                stale.append(name)
        if stale:
            print("stale content: " + ", ".join(sorted(stale)[:8]) +
                  (" ..." if len(stale) > 8 else ""), file=sys.stderr)
            return 1
        print(f"{len(files)} content files match the jar", file=sys.stderr)
        return 0
    for directory in ["assets", "data"]:
        shutil.rmtree(CONTENT / directory, ignore_errors=True)
    for name, data in files.items():
        path = CONTENT / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    print(f"{len(files)} content files written", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
