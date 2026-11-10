#!/usr/bin/env python3
"""Resolve the item tags the Theurgy calcination recipes name, once, into JSON.

The pinned engine exposes no item-tag membership helper, so the port's runtime
carries explicit member lists. This script builds them and records where each
came from:

  * ``minecraft:*`` — the prepared 26.3 tag files (authoritative for 26.3),
  * ``c:*``         — NeoForge's generated ``c:`` tags for 1.21.x (the mod's own
                      runtime: ``theurgy`` depends on NeoForge, which ships them),
  * ``theurgy:*``   — the jar's own ``data/theurgy/tags/item/*.json``.

Nested ``#tag`` members are followed recursively. Output: ``tools/tag_members.json``
committed next to this script, so later runs (and the build) need no network.

    python3 tools/gen_tag_members.py            # resolve, write the JSON
    python3 tools/gen_tag_members.py --check    # re-resolve and diff, no write
"""
import argparse
import json
from pathlib import Path
import sys
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
JAR = ROOT.parent / "_shared/licence-cache/theurgy-1.21.1-neoforge-1.78.0.jar"
RES_263 = Path("${HORIZON_RES:-$HOME/.local/horizon-client/26.3}")
NEOFORGE = "https://raw.githubusercontent.com/neoforged/NeoForge/1.21.x/src/generated/resources/data"
OUT = ROOT / "tools/tag_members.json"

# The tags the calcination recipes name (jar `data/theurgy/recipe/calcination/`).
REQUESTED = [
    "minecraft:leaves",
    "minecraft:logs",
    "minecraft:saplings",
    "minecraft:sand",
    "minecraft:dirt",
    "c:gems",
    "c:ingots",
    "c:ores",
    "c:raw_materials",
    "c:cobblestones",
    "c:concretes",
    "c:stones",
    "c:netherracks",
    "c:sandstone/blocks",
    "c:crops",
    "theurgy:other_minerals",
    # Named by the two crafting recipes (`data/theurgy/recipe/{calcination_oven,
    # pyromantic_brazier}.json`), not by a calcination recipe.
    "c:ingots/copper",
    "c:ingots/iron",
    "c:storage_blocks/copper",
]


def values_of(document):
    """The ids a tag file's ``values`` list names, in order (§ tags are kept)."""
    out = []
    for entry in document.get("values", []):
        if isinstance(entry, str):
            out.append(entry)
        elif isinstance(entry, dict) and "id" in entry:
            if entry.get("required", True):
                out.append(entry["id"])
    return out


def vanilla_tag(name):
    path = RES_263 / "data/minecraft/tags/item" / (name.removeprefix("minecraft:") + ".json")
    if not path.is_file():
        return None
    return values_of(json.loads(path.read_text()))


def jar_tag(name):
    with zipfile.ZipFile(JAR) as jar:
        try:
            raw = jar.read(f"data/{name.split(':')[0]}/tags/item/{name.split(':', 1)[1]}.json")
        except KeyError:
            return None
    return values_of(json.loads(raw))


_cache = {}


def neoforge_tag(name):
    if name in _cache:
        return _cache[name]
    namespace, path = name.split(":", 1)
    url = f"{NEOFORGE}/{namespace}/tags/item/{path}.json"
    try:
        with urllib.request.urlopen(url, timeout=30) as reply:
            document = json.loads(reply.read())
    except Exception:  # a tag NeoForge does not ship: an empty list, recorded
        _cache[name] = None
        return None
    _cache[name] = values_of(document)
    return _cache[name]


def members(name, seen):
    if name in seen:
        return []
    seen = seen | {name}
    namespace = name.split(":", 1)[0]
    if namespace == "minecraft":
        listed = vanilla_tag(name)
        source = "26.3 vanilla tag"
    elif namespace == "c":
        listed = neoforge_tag(name)
        source = "NeoForge 1.21.x generated c: tag"
    else:
        listed = jar_tag(name)
        source = "jar data/theurgy/tags/item"
    if listed is None:
        return [], source, False
    out = []
    for entry in listed:
        if entry.startswith("#"):
            deeper, _, _ = members(entry[1:], seen)
            out.extend(deeper)
        else:
            out.append(entry)
    return out, source, True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    table = {}
    for tag in REQUESTED:
        resolved, source, known = members(tag, frozenset())
        table[tag] = {"source": source, "known": known,
                      "members": sorted(set(resolved))}
        print(f"{tag:28} {len(table[tag]['members']):3} members  ({source})", file=sys.stderr)
    text = json.dumps(table, indent=1, sort_keys=True) + "\n"
    if args.check:
        current = OUT.read_text() if OUT.is_file() else ""
        if current != text:
            print(f"{OUT} is stale: re-run without --check", file=sys.stderr)
            return 1
        return 0
    OUT.write_text(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
