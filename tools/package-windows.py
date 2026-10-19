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


def stt_target():
    """The worker builds in a short directory: whisper.cpp's Vulkan backend nests a CMake
    project for its shader generator deep enough to pass MSBuild's 260-character path limit
    under target/release/build/."""
    return Path(os.environ.get("MACAFON_STT_TARGET", Path(ROOT.anchor) / "mst"))


def vulkan_env():
    """The Vulkan SDK (headers, vulkan-1.lib, glslc) for the GPU build of Whisper (STT-020):
    VULKAN_SDK, else a local SDK folder; CMake and libclang from Visual Studio or LIBCLANG_PATH."""
    env = dict(os.environ)
    sdk = env.get("VULKAN_SDK") or str(Path(env["LOCALAPPDATA"]) / "macafon-vulkan-sdk")
    if not (Path(sdk) / "Bin/glslc.exe").is_file():
        sys.exit(f"The Vulkan SDK is required to build GPU dictation (glslc not found in {sdk}); "
                 "set VULKAN_SDK")
    env["VULKAN_SDK"] = sdk
    # MSBuild's tracking logs pass the 260-character path limit inside ggml's nested shader
    # generator project; Ninja does not write them. It needs the MSVC environment.
    env.update(msvc_env())
    env["CMAKE_GENERATOR"] = "Ninja"
    env["PATH"] = os.pathsep.join([str(Path(sdk) / "Bin"), env.get("PATH", "")])
    return env


def msvc_env():
    """The x64 MSVC developer environment (cl, link, ninja, cmake) from vcvars64.bat."""
    vswhere = Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")) / \
        "Microsoft Visual Studio/Installer/vswhere.exe"
    install = subprocess.check_output(
        [str(vswhere), "-latest", "-products", "*", "-requires",
         "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"],
        text=True).strip()
    vcvars = Path(install) / "VC/Auxiliary/Build/vcvars64.bat"
    output = subprocess.check_output(f'"{vcvars}" >nul && set', shell=True, text=True)
    # cmd preserves names such as Path; ordinary Python dicts are case-sensitive.
    env = {key.upper(): value for line in output.splitlines() if "=" in line
           for key, value in [line.split("=", 1)]}
    cmake = Path(install) / "Common7/IDE/CommonExtensions/Microsoft/CMake"
    env["PATH"] = os.pathsep.join([str(cmake / "Ninja"), str(cmake / "CMake/bin"), env["PATH"]])
    return env


def build_worker():
    target = stt_target()
    executable = shutil.which("cargo")
    subprocess.run([executable, "build", "--release", "--locked", "-p", "macafon-stt-worker",
                    "--target-dir", str(target)], cwd=ROOT, check=True, env=vulkan_env())
    release = ROOT / "target/release"
    release.mkdir(parents=True, exist_ok=True)
    for name in ("macafon-stt.exe", "sherpa-onnx-c-api.dll", "onnxruntime.dll",
                 "onnxruntime_providers_shared.dll"):
        shutil.copy2(target / "release" / name, release / name)


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
        run("cargo", "build", "--release", "--locked", "-p", "macafond", "-p", "macafon-cli")
        build_worker()
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
