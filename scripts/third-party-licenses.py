#!/usr/bin/env python3
"""Write THIRD_PARTY_LICENSES.txt: the license of everything the released `moon` is built from.

`moon licenses` prints that file, which is compiled into the executable - an install through
install.sh keeps nothing but the executable, so that is the one place the notices can travel
with it.

What goes in:

- every crate the release targets link, found by walking `cargo metadata`'s resolve from the
  `moon-review` package through its normal and build dependencies (never dev ones), with the
  license files each crate ships;
- what reaches the executable without being a crate, listed in NOT_CRATES below.

A license text several crates ship word for word is written once, under all of them.

    scripts/third-party-licenses.py          write the file
    scripts/third-party-licenses.py --check  fail when the file is not what would be written
"""

import json
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "THIRD_PARTY_LICENSES.txt"
ROOT_PACKAGE = "moon-review"

# The targets the release is built for - see scripts/_internal/build-release.sh.
RELEASE_TARGETS = [
    "aarch64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
]

# A crate's files that are its license, by the start of their name.
LICENSE_FILE_PREFIXES = ("license", "licence", "copying", "notice", "unlicense")

# The standard text of each license a crate with no license file of its own is under, by SPDX id,
# out of https://github.com/spdx/license-list-data.
LICENSE_TEXTS = ROOT / "scripts" / "license-texts"
# The words of a license expression that name no license.
EXPRESSION_WORDS = {"OR", "AND", "WITH"}

# What is compiled into the executable without being a crate: what it is, and its license files
# relative to the repo root.
NOT_CRATES = [
    (
        "OpenAI Codex - the inline visualization stylesheet and runtime in "
        "assets/codex_visualization/, and the visualization viewer and directive rules they come with",
        ["assets/codex_visualization/LICENSE", "assets/codex_visualization/NOTICE"],
    ),
    ("Hack font", ["assets/fonts/Hack-LICENSE.md"]),
    (
        "Sublime Text syntax definitions in crates/egui_moon_editor/grammars/",
        ["crates/egui_moon_editor/grammars/LICENSE-Apache-2.0.txt"],
    ),
]

# The dependency kinds that end up in the executable.
LINKED_KINDS = {None, "build"}


def metadata_for(target):
    output = subprocess.run(
        [
            "cargo",
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--filter-platform",
            target,
        ],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return json.loads(output)


def linked_packages(metadata):
    """The ids of every package the root package links, the root left out."""
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    root = next(
        package["id"]
        for package in metadata["packages"]
        if package["name"] == ROOT_PACKAGE and package["source"] is None
    )
    linked = set()
    waiting = [root]
    while waiting:
        node = nodes[waiting.pop()]
        for dependency in node["deps"]:
            if not any(kind["kind"] in LINKED_KINDS for kind in dependency["dep_kinds"]):
                continue
            if dependency["pkg"] not in linked:
                linked.add(dependency["pkg"])
                waiting.append(dependency["pkg"])
    return linked


def license_files(package):
    folder = Path(package["manifest_path"]).parent
    named = [folder / package["license_file"]] if package.get("license_file") else []
    found = sorted(
        path
        for path in folder.iterdir()
        if path.is_file() and path.name.lower().startswith(LICENSE_FILE_PREFIXES)
    )
    return list(dict.fromkeys(named + found))


def license_ids(expression):
    """The SPDX ids a license expression names: `(MIT OR Apache-2.0) AND OFL-1.1`, or the older
    `MIT/Apache-2.0`."""
    words = expression.replace("(", " ").replace(")", " ").replace("/", " ").split()
    return [word for word in words if word not in EXPRESSION_WORDS]


def read(path):
    return path.read_text(encoding="utf-8", errors="replace").strip()


def rendered():
    packages = {}
    for target in RELEASE_TARGETS:
        metadata = metadata_for(target)
        by_id = {package["id"]: package for package in metadata["packages"]}
        for package_id in linked_packages(metadata):
            packages[package_id] = by_id[package_id]

    # Each license file's text, once, under every crate that ships it: the Apache text most
    # crates carry is word for word the same, while the MIT texts differ by their copyright line.
    crates_by_text = defaultdict(list)
    without_files = []
    for package in sorted(packages.values(), key=lambda package: (package["name"], package["version"])):
        crate = f"{package['name']} {package['version']} ({package.get('license') or 'no license given'})"
        files = license_files(package)
        if not files:
            without_files.append((crate, package["license"]))
            continue
        for path in files:
            crates_by_text[read(path)].append(crate)

    rule = "=" * 100
    sections = [
        "Third-party licenses of moon\n\n"
        "moon is MIT licensed - see its LICENSE. It is built from the work below, whose licenses\n"
        "and notices follow. Generated by scripts/third-party-licenses.py; do not edit by hand."
    ]
    for what, paths in NOT_CRATES:
        texts = "\n\n".join(f"--- {Path(path).name}\n\n{read(ROOT / path)}" for path in paths)
        sections.append(f"{rule}\n{what}\n{rule}\n\n{texts}")
    for text, crates in sorted(crates_by_text.items(), key=lambda entry: entry[1][0]):
        sections.append(f"{rule}\n" + "\n".join(crates) + f"\n{rule}\n\n{text}")
    if without_files:
        sections.append(
            f"{rule}\nCrates that ship no license file, under the license their manifest names\n{rule}\n\n"
            + "\n".join(crate for crate, _ in without_files)
        )
        # A license named here with no text in LICENSE_TEXTS is a text to add there, not one to
        # leave out - so it stops the script.
        named = sorted({license_id for _, expression in without_files for license_id in license_ids(expression)})
        for license_id in named:
            sections.append(f"{rule}\n{license_id}\n{rule}\n\n{read(LICENSE_TEXTS / f'{license_id}.txt')}")
    return "\n\n\n".join(sections) + "\n"


def main():
    text = rendered()
    if sys.argv[1:] == ["--check"]:
        if not OUTPUT.exists() or OUTPUT.read_text(encoding="utf-8") != text:
            print(
                f"{OUTPUT.name} is out of date: run scripts/third-party-licenses.py",
                file=sys.stderr,
            )
            sys.exit(1)
        return
    if sys.argv[1:]:
        sys.exit(__doc__)
    OUTPUT.write_text(text, encoding="utf-8")
    print(f"wrote {OUTPUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
