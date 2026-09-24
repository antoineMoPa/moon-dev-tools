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

A license's wording is written once, under every crate whose license file carries it, each crate
with the copyright lines of its own file: the Apache and MIT texts differ between crates only by
who holds the copyright, and by spacing.

    scripts/third-party-licenses.py          write the file
    scripts/third-party-licenses.py --check  fail when the file is not what would be written
"""

import json
import re
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
    # The browser build of the window, which each executable embeds - see build.rs.
    "wasm32-unknown-unknown",
]

# A crate's files that are its license, by the start of their name.
LICENSE_FILE_PREFIXES = ("license", "licence", "copying", "notice", "unlicense")

# The standard text of each license a crate with no license file of its own is under, by SPDX id,
# out of https://github.com/spdx/license-list-data.
LICENSE_TEXTS = ROOT / "scripts" / "license-texts"
# The words of a license expression that name no license.
EXPRESSION_WORDS = {"OR", "AND", "WITH"}

# The start of a line that says who holds the copyright - "Copyright (c) 2015 Jane Doe",
# "(c) 2018 The Foo Authors", "© 2020 Bar" - as opposed to the license's own wording about
# copyright: "copyright license to reproduce", "Copyright and related rights", "(c) You must".
COPYRIGHT_LINE = re.compile(
    r"^(copyright\b(?!\s+(license|notice|owner|holder|and|or|in|to|of)\b)|\(c\)\s*\d{4}|©)", re.I
)
# What marks a copyright line as the license's fill-in template rather than a holder's.
COPYRIGHT_TEMPLATE_MARKS = ("[yyyy]", "[year]", "<year>", "<yyyy>")

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


def is_copyright_line(line):
    return bool(COPYRIGHT_LINE.match(line)) and not any(
        mark in line.lower() for mark in COPYRIGHT_TEMPLATE_MARKS
    )


def split_license_file(text):
    """A license file's lines that say who holds the copyright; the rest, the license's wording,
    as written; and that wording with its spacing evened out, the same wherever crates lay the
    same license out differently."""
    copyrights, wording, key = [], [], []
    for line in text.splitlines():
        flat = " ".join(line.split())
        if is_copyright_line(flat):
            copyrights.append(flat)
        else:
            wording.append(line.rstrip())
            if flat:
                key.append(flat)
    return copyrights, "\n".join(wording).strip("\n"), "\n".join(key)


def rendered():
    packages = {}
    for target in RELEASE_TARGETS:
        metadata = metadata_for(target)
        by_id = {package["id"]: package for package in metadata["packages"]}
        for package_id in linked_packages(metadata):
            packages[package_id] = by_id[package_id]

    # Each license's wording, once, under every crate whose license file carries it, with that
    # file's copyright lines: keyed by the wording with its spacing evened out, holding the
    # wording as the first crate's file writes it.
    crates_by_wording = defaultdict(list)
    wording_by_key = {}
    without_files = []
    for package in sorted(packages.values(), key=lambda package: (package["name"], package["version"])):
        crate = f"{package['name']} {package['version']} ({package.get('license') or 'no license given'})"
        files = license_files(package)
        if not files:
            without_files.append((crate, package["license"]))
            continue
        for path in files:
            copyrights, wording, key = split_license_file(read(path))
            wording_by_key.setdefault(key, wording)
            crates_by_wording[key].append("\n".join([crate] + [f"    {line}" for line in copyrights]))

    rule = "=" * 100
    sections = [
        "Third-party licenses of moon\n\n"
        "moon is MIT licensed - see its LICENSE. It is built from the work below, whose licenses\n"
        "and notices follow. Generated by scripts/third-party-licenses.py; do not edit by hand."
    ]
    for what, paths in NOT_CRATES:
        texts = "\n\n".join(f"--- {Path(path).name}\n\n{read(ROOT / path)}" for path in paths)
        sections.append(f"{rule}\n{what}\n{rule}\n\n{texts}")
    for key, crates in sorted(crates_by_wording.items(), key=lambda entry: entry[1][0]):
        sections.append(f"{rule}\n" + "\n".join(crates) + f"\n{rule}\n\n{wording_by_key[key]}")
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
