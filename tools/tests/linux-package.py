#!/usr/bin/env python3
"""Linux packaging regressions; never operate on the installed user service."""
import importlib.util
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("package_linux", ROOT / "tools/package-linux.py")
PACKAGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PACKAGE)


class LinuxPackaging(unittest.TestCase):
    def test_builds_use_bundled_cpu_bindings_without_changing_parent_environment(self):
        with tempfile.TemporaryDirectory() as directory, \
                patch.dict(os.environ, {"WHISPER_DONT_GENERATE_BINDINGS": "parent"}), \
                patch.object(PACKAGE, "ROOT", Path(directory)), \
                patch.object(PACKAGE.shutil, "which", return_value="pnpm"), \
                patch.object(PACKAGE.subprocess, "check_output", return_value="host: x86_64-unknown-linux-gnu\n"), \
                patch.object(PACKAGE, "notices"), patch.object(PACKAGE, "stt_notices"), \
                patch.object(PACKAGE, "configuration", return_value={}), \
                patch.object(PACKAGE, "run") as run, \
                patch("sys.argv", ["package-linux.py", "--build-only"]):
            PACKAGE.main()
            self.assertEqual(len(run.call_args_list), 2)
            self.assertEqual(run.call_args_list[0].args[:2], ("cargo", "build"))
            self.assertEqual(run.call_args_list[1].args[:3], ("pnpm", "tauri", "build"))
            for call in run.call_args_list:
                self.assertEqual(call.kwargs["env"]["WHISPER_DONT_GENERATE_BINDINGS"], "1")
            self.assertEqual(os.environ["WHISPER_DONT_GENERATE_BINDINGS"], "parent")



if __name__ == "__main__":
    unittest.main()
