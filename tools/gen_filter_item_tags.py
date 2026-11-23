#!/usr/bin/env python3
"""Writes data/theurgy/filter_item_tags.json: the item tag names the attribute filter screen
offers for a reference item.

The mod API has no "tags of this item" read, so the screen tests the reference item against a
known list: vanilla's item tags (from the game jar given as the only argument) and every item
tag this package carries under data/*/tags/item. Run from the package root:

    python tools/gen_filter_item_tags.py <path to 26.3.jar>
"""
import json
import os
import sys
import zipfile

VANILLA = "data/minecraft/tags/item/"


def main() -> None:
    names = set()
    with zipfile.ZipFile(sys.argv[1]) as jar:
        for name in jar.namelist():
            if name.startswith(VANILLA) and name.endswith(".json"):
                names.add("minecraft:" + name[len(VANILLA):-5])
    for root, _, files in os.walk("data"):
        parts = root.replace(os.sep, "/").split("/")
        if len(parts) < 4 or parts[2:4] != ["tags", "item"]:
            continue
        prefix = "/".join(parts[4:])
        for file in files:
            if file.endswith(".json"):
                path = f"{prefix}/{file[:-5]}" if prefix else file[:-5]
                names.add(f"{parts[1]}:{path}")
    with open("data/theurgy/filter_item_tags.json", "w", encoding="utf-8", newline="\n") as out:
        json.dump(sorted(names), out, indent=0)
        out.write("\n")
    print(len(names), "item tags")


if __name__ == "__main__":
    main()
