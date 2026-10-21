#!/usr/bin/env python3
"""Bump the Macafon version everywhere it lives (AGENTS.md §2, "Versions").

    python tools/bump-version.py patch   # 0.4.0 -> 0.4.1: fixes and small changes
    python tools/bump-version.py minor   # 0.4.1 -> 0.5.0: features and large changes
    python tools/bump-version.py 1.0.0   # an explicit version (major releases: owner only)

Updates the workspace Cargo.toml, the UI package.json, tauri.conf.json and the workspace
entries of Cargo.lock; prints the new version.
"""

import argparse
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
CARGO = ROOT / "Cargo.toml"
LOCK = ROOT / "Cargo.lock"
JSON_FILES = [ROOT / "apps/macafon-ui/package.json", ROOT / "apps/macafon-ui/src-tauri/tauri.conf.json"]
SEMVER = re.compile(r"^(\d+)\.(\d+)\.(\d+)$")
CARGO_VERSION = re.compile(r'(\[workspace\.package\][^\[]*?\nversion = ")([^"]+)(")')
JSON_VERSION = re.compile(r'^(  "version": ")([^"]+)(",?)$', re.M)


def current_version():
    match = CARGO_VERSION.search(CARGO.read_text(encoding="utf-8"))
    if not match:
        sys.exit("version not found in [workspace.package] of Cargo.toml")
    return match.group(2)


def next_version(current, bump):
    if SEMVER.match(bump):
        return bump
    major, minor, patch = map(int, SEMVER.match(current).groups())
    if bump == "minor":
        return f"{major}.{minor + 1}.0"
    return f"{major}.{minor}.{patch + 1}"


def replace(path, pattern, version, expected):
    text = path.read_text(encoding="utf-8")
    found = pattern.search(text)
    if not found or found.group(2) != expected:
        sys.exit(f"{path.relative_to(ROOT)}: expected version {expected}, "
                 f"found {found.group(2) if found else 'none'}")
    path.write_bytes(pattern.sub(rf"\g<1>{version}\g<3>", text, count=1).encode("utf-8"))


def update_lock(current, version):
    """Workspace members are the lock entries without a `source`; their version follows the
    workspace one."""
    text = LOCK.read_text(encoding="utf-8")
    packages = text.split("\n[[package]]\n")
    for index, package in enumerate(packages):
        if "\nsource = " not in package and f'\nversion = "{current}"\n' in package:
            packages[index] = package.replace(f'\nversion = "{current}"\n', f'\nversion = "{version}"\n')
    LOCK.write_bytes("\n[[package]]\n".join(packages).encode("utf-8"))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("bump", help="patch, minor or an explicit X.Y.Z")
    bump = parser.parse_args().bump
    if bump not in ("patch", "minor") and not SEMVER.match(bump):
        sys.exit("expected patch, minor or X.Y.Z")
    current = current_version()
    version = next_version(current, bump)
    if version == current:
        sys.exit(f"already {current}")
    replace(CARGO, CARGO_VERSION, version, current)
    for path in JSON_FILES:
        replace(path, JSON_VERSION, version, current)
    update_lock(current, version)
    print(f"{current} -> {version}")


if __name__ == "__main__":
    main()
