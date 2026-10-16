#!/usr/bin/env python3
"""Build the per-user Windows installer locally using the Tauri bundler; never publish it."""

import argparse
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def linux_packaging():
    """`notices` is shared with the Linux packages."""
    spec = importlib.util.spec_from_file_location("package_linux", ROOT / "tools/package-linux.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def run(program, *args, cwd=ROOT):
    # pnpm comes from corepack on Windows; a full path also runs its .cmd shim.
    executable = shutil.which(program)
    if not executable:
        sys.exit(f"{program} is required")
    subprocess.run([executable, *map(str, args)], cwd=cwd, check=True)


def configuration(stage, triple):
    sidecars = []
    for name in ("macafond", "macafon"):
        shutil.copy2(ROOT / f"target/release/{name}.exe", stage / f"{name}-{triple}.exe")
        sidecars.append(str(stage / name))
    return {
        # Windows names are case-insensitive: "Macafon.exe" would replace the CLI macafon.exe.
        "mainBinaryName": "macafon-ui",
        # pnpm is usually reached through corepack here, not on PATH.
        "build": {"beforeBuildCommand": "corepack pnpm build"},
        "bundle": {
            "active": True,
            "targets": ["nsis"],
            "publisher": "Mar1kLab",
            "homepage": "https://github.com/Mar1kLab/Macafon",
            "category": "Music",
            "license": "GPL-3.0-only",
            "licenseFile": str(ROOT / "LICENSE"),
            "shortDescription": "Microphone processing with a virtual microphone",
            "externalBin": sidecars,
            "resources": {str(stage / "licenses"): "licenses"},
            "windows": {
                "nsis": {
                    # No administrator rights: no services, drivers or machine-wide files.
                    "installMode": "currentUser",
                    "installerHooks": str(ROOT / "packaging/windows/hooks.nsh"),
                },
            },
        },
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle-only", action="store_true", help="use existing release binaries")
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("Windows is required")
    triple = next(line.split(": ", 1)[1] for line in subprocess.check_output(
        ["rustc", "-vV"], text=True).splitlines() if line.startswith("host:"))
    stage = ROOT / "target/package-windows"
    stage.mkdir(parents=True, exist_ok=True)
    if not args.bundle_only:
        run("cargo", "build", "--release", "--locked", "-p", "macafond", "-p", "macafon-cli")
    linux_packaging().notices(stage / "licenses")
    config = stage / "tauri-bundle.conf.json"
    config.write_text(json.dumps(configuration(stage, triple), indent=2) + "\n")
    ui = ROOT / "apps/macafon-ui"
    if not args.bundle_only:
        run("corepack", "pnpm", "tauri", "build", "--no-bundle", "--config", config, cwd=ui)
    run("corepack", "pnpm", "tauri", "bundle", "--bundles", "nsis", "--config", config, cwd=ui)
    for installer in (ROOT / "target/release/bundle/nsis").glob("*.exe"):
        print(installer)


if __name__ == "__main__":
    main()
