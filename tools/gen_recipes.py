#!/usr/bin/env python3
"""Generate the calcination recipe table from the jar, and check it has not drifted.

The pinned content schema has no custom recipe types (docs/modpacks.md: machine
recipes arrive later), so the port carries the jar's `theurgy:calcination`
recipes as data in `mixins/theurgy/runtime.rs`, between the two marker lines
this script owns. Every value comes from `data/theurgy/recipe/calcination/*.json`;
nothing is typed by hand.

    python3 tools/gen_recipes.py            # write tools/recipes.json + runtime.rs
    python3 tools/gen_recipes.py --check    # fail if either is stale
"""
import argparse
import json
from pathlib import Path
import sys
import zipfile

ROOT = Path(__file__).resolve().parents[1]
JAR = ROOT.parent / "_shared/licence-cache/theurgy-1.21.1-neoforge-1.78.0.jar"
TAGS = ROOT / "tools/tag_members.json"
OUT = ROOT / "tools/recipes.json"
RUNTIME = ROOT / "mixins/theurgy/runtime.rs"
BEGIN = "// BEGIN GENERATED CALCINATION RECIPES (tools/gen_recipes.py)"
END = "// END GENERATED CALCINATION RECIPES"


def recipes():
    members = json.loads(TAGS.read_text())
    out = []
    with zipfile.ZipFile(JAR) as jar:
        names = sorted(n for n in jar.namelist()
                       if n.startswith("data/theurgy/recipe/calcination/") and n.endswith(".json"))
        for name in names:
            document = json.loads(jar.read(name))
            if document.get("type") != "theurgy:calcination":
                raise SystemExit(f"{name} is not a theurgy:calcination recipe")
            ingredient = document["ingredient"]
            inner = ingredient["ingredient"]
            if "item" in inner:
                source = {"kind": "item", "items": [inner["item"]]}
            elif "tag" in inner:
                tag = inner["tag"]
                if tag not in members:
                    raise SystemExit(f"{name} names tag {tag}, not in tools/tag_members.json")
                source = {"kind": "tag", "tag": tag, "items": members[tag]["members"]}
            else:
                raise SystemExit(f"{name} has an ingredient this script does not read: {inner}")
            out.append({
                "file": name.rsplit("/", 1)[-1],
                "input": source,
                "count": ingredient["count"],
                "output": document["result"]["id"],
                "output_count": document["result"]["count"],
                "time": document.get("time", 100),
            })
    return out


def rust(table):
    """The `pub static RECIPES` block, deterministic, one recipe per line."""
    lines = [BEGIN, "", "/// Every `theurgy:calcination` recipe the jar ships, from",
             "/// `data/theurgy/recipe/calcination/*.json` (86 files, all `time: 100`).",
             "/// A tag ingredient keeps the tag the jar names; the engine's merged item tags",
             "/// answer its membership (`native_server_item_in_tag`, API 63).",
             "#[rustfmt::skip]",
             "pub static RECIPES: &[Recipe] = &["]
    for entry in table:
        source = entry["input"]
        if source["kind"] == "item":
            ingredient = f'Ingredient::Item("{source["items"][0]}")'
        else:
            ingredient = f'Ingredient::Tag("{source["tag"]}")'
        lines.append(
            f'    Recipe {{ input: {ingredient}, count: {entry["count"]}, '
            f'output: "{entry["output"]}", output_count: {entry["output_count"]}, '
            f'time: {entry["time"]} }}, // {entry["file"]}')
    lines.append("];")
    lines.append(END)
    return "\n".join(lines) + "\n"


def splice(text, block):
    begin = text.index(BEGIN)
    end = text.index(END) + len(END) + 1
    return text[:begin] + block + text[end:]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    table = recipes()
    document = json.dumps(table, indent=1) + "\n"
    runtime = RUNTIME.read_text() if RUNTIME.is_file() else ""
    spliced = splice(runtime, rust(table)) if BEGIN in runtime else None
    if args.check:
        stale = []
        if not OUT.is_file() or OUT.read_text() != document:
            stale.append(str(OUT))
        if spliced is None or spliced != runtime:
            stale.append(str(RUNTIME))
        if stale:
            print("stale: " + ", ".join(stale) + " (re-run without --check)", file=sys.stderr)
            return 1
        print(f"{len(table)} recipes match the jar", file=sys.stderr)
        return 0
    OUT.write_text(document)
    if spliced is None:
        print(f"{RUNTIME} has no generated-recipe markers yet", file=sys.stderr)
        return 1
    RUNTIME.write_text(spliced)
    print(f"{len(table)} recipes written", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
