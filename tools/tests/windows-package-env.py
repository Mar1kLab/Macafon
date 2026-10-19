#!/usr/bin/env python3
"""Regression: vcvars64 may return Path with mixed casing on Windows runners."""

import importlib.util
import os
from pathlib import Path
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("package_windows", ROOT / "tools/package-windows.py")
PACKAGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PACKAGE)


class MsvcEnvironment(unittest.TestCase):
    def test_mixed_case_path_is_available_to_the_worker(self):
        output = "Path=C:\\compiler\\bin;C:\\Windows\\System32\nInclude=C:\\headers\n"
        with patch.object(PACKAGE.subprocess, "check_output", side_effect=["C:/VS\n", output]):
            env = PACKAGE.msvc_env()
        self.assertEqual(
            env["PATH"],
            os.pathsep.join([
                str(Path("C:/VS/Common7/IDE/CommonExtensions/Microsoft/CMake/Ninja")),
                str(Path("C:/VS/Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin")),
                "C:\\compiler\\bin;C:\\Windows\\System32",
            ]),
        )
        self.assertEqual(env["INCLUDE"], "C:\\headers")
        self.assertNotIn("Path", env)


if __name__ == "__main__":
    unittest.main()
