#!/usr/bin/env python3
"""Build the per-user Windows installer locally using the Tauri bundler; never publish it.

`--install` also installs or upgrades Macafon for the current user: no prompts, no UAC.
"""

import argparse
import importlib.util
import json
import os
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
    for name in ("macafond", "macafon", "macafon-stt"):
        shutil.copy2(ROOT / f"target/release/{name}.exe", stage / f"{name}-{triple}.exe")
        sidecars.append(str(stage / name))
    resources = {str(stage / "licenses"): "licenses"}
    # sherpa-onnx's build script stages the matching shared runtime beside the exe.
    # Explicit names prevent accidentally shipping unrelated DLLs from target/release.
    runtime_names = ("sherpa-onnx-c-api.dll", "onnxruntime.dll",
                     "onnxruntime_providers_shared.dll")
    for name in runtime_names:
        source = ROOT / "target/release" / name
        if not source.is_file():
            sys.exit(f"Missing dictation runtime: {source}; rebuild macafon-stt-worker")
        shutil.copy2(source, stage / name)
        resources[str(stage / name)] = name
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
            "resources": resources,
            "windows": {
                "nsis": {
                    # No administrator rights: no services, drivers or machine-wide files.
                    "installMode": "currentUser",
                    "installerHooks": str(ROOT / "packaging/windows/hooks.nsh"),
                },
            },
        },
    }


def stt_notices(destination):
    """Native runtimes are not separate Cargo packages; include their notices too."""
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--format-version", "1"], cwd=ROOT, text=True))
    whisper_sys = next(p for p in metadata["packages"] if p["name"] == "whisper-rs-sys")
    source = Path(whisper_sys["manifest_path"]).parent / "whisper.cpp/LICENSE"
    shutil.copyfile(source, destination / "whisper.cpp-LICENSE.txt")
    # https://github.com/microsoft/onnxruntime/blob/main/LICENSE (MIT).
    (destination / "onnxruntime-LICENSE.txt").write_text("""MIT License

Copyright (c) Microsoft Corporation

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:
The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.
THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
""", encoding="utf-8")
    (destination / "dictation-models.txt").write_text("""Models are downloaded separately, never bundled.
Whisper: OpenAI, MIT; ggml conversion/quantization by whisper.cpp contributors.
https://huggingface.co/ggerganov/whisper.cpp
Silero VAD: Silero Team, MIT; GGML conversion by whisper.cpp contributors.
https://github.com/snakers4/silero-vad
Parakeet TDT 0.6B v3: NVIDIA, CC-BY-4.0.
https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3
https://creativecommons.org/licenses/by/4.0/
ONNX export and int8 quantization by the sherpa-onnx contributors:
https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8
""", encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle-only", action="store_true", help="use existing release binaries")
    parser.add_argument("--install", action="store_true",
                        help="install or upgrade for the current user after building")
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("Windows is required")
    triple = next(line.split(": ", 1)[1] for line in subprocess.check_output(
        ["rustc", "-vV"], text=True).splitlines() if line.startswith("host:"))
    stage = ROOT / "target/package-windows"
    stage.mkdir(parents=True, exist_ok=True)
    if not args.bundle_only:
        # The project-specific hook in .cargo/config.toml configures Whisper's
        # optimization and portable ISA equally for normal and installer builds.
        run("cargo", "build", "--release", "--locked", "-p", "macafond", "-p", "macafon-cli",
            "-p", "macafon-stt-worker")
    linux_packaging().notices(stage / "licenses")
    stt_notices(stage / "licenses")
    config = stage / "tauri-bundle.conf.json"
    config.write_text(json.dumps(configuration(stage, triple), indent=2) + "\n")
    ui = ROOT / "apps/macafon-ui"
    if not args.bundle_only:
        run("corepack", "pnpm", "tauri", "build", "--no-bundle", "--config", config, cwd=ui)
    run("corepack", "pnpm", "tauri", "bundle", "--bundles", "nsis", "--config", config, cwd=ui)
    version = json.loads((ui / "src-tauri/tauri.conf.json").read_text())["version"]
    installer = ROOT / f"target/release/bundle/nsis/Macafon_{version}_x64-setup.exe"
    print(installer)
    if args.install:
        install(installer)


def install(installer):
    """Passive mode: a progress window only; the hooks stop and restart a running daemon."""
    first = not (Path(os.environ["LOCALAPPDATA"]) / "Macafon/macafon-ui.exe").is_file()
    # /R opens the UI after a first install. /UPDATE copies over the old version: without it a
    # passive reinstall runs the old uninstaller, which restores audio and drops autostart.
    subprocess.run([str(installer), "/P", "/R" if first else "/UPDATE"], check=True)
    print("installed" if first else "upgraded")


if __name__ == "__main__":
    main()
