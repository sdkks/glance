import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "generate-notices.py"
spec = importlib.util.spec_from_file_location("notices", SCRIPT)
notices = importlib.util.module_from_spec(spec)
spec.loader.exec_module(notices)


class NoticesTests(unittest.TestCase):
    def test_policy_boolean_semantics(self):
        allowed = {"MIT", "Apache-2.0", "Unicode-3.0"}
        for expression in ("MIT/Apache-2.0", "GPL-2.0-only OR MIT",
                           "(MIT OR Apache-2.0) AND Unicode-3.0",
                           "Apache-2.0 WITH LLVM-exception OR MIT"):
            self.assertTrue(notices.permitted(expression, allowed))
        for expression in ("MIT AND GPL-2.0-only", "GPL-2.0-only",
                           "Apache-2.0 WITH LLVM-exception"):
            self.assertFalse(notices.permitted(expression, allowed))
        self.assertTrue(notices.permitted("Apache-2.0 WITH LLVM-exception",
                                         {"Apache-2.0 WITH LLVM-exception"}))

    def test_malformed_expressions_fail_closed(self):
        for expression in ("", "MIT OR", "MIT AND", "(MIT", "MIT)", "MIT WITH",
                           "MIT + ???", "MIT MIT", "MIT WITH OR"):
            with self.subTest(expression=expression), self.assertRaises(ValueError):
                notices.permitted(expression, {"MIT"})

    def test_bundle_and_cli_drift_check(self):
        with tempfile.TemporaryDirectory(prefix="glance-notices-test-") as temp:
            root = Path(temp)
            (root / "LICENSE").write_text("Project MIT notice\n")
            fonts = root / "licenses/fonts"
            fonts.mkdir(parents=True)
            (fonts / "OFL.txt").write_text("Font notice\n")
            crate = root / "registry/sample-1.0.0"
            crate.mkdir(parents=True)
            (crate / "LICENSE-MIT").write_text("Copyright Sample; MIT terms\n")
            (crate / "NOTICE").write_text("Additional attribution\n")
            (crate / "lib.rs").write_text("DO NOT COPY SOURCE")
            policy = root / "deny.toml"
            policy.write_text('[licenses]\nallow = ["MIT"]\n')
            package = {"name": "sample", "version": "1.0.0", "id": "sample",
                       "license": "MIT", "manifest_path": str(crate / "Cargo.toml"),
                       "repository": "https://example.org/sample", "source": "registry+test"}
            metadata = {"packages": [package], "workspace_members": ["glance"]}
            native = root / "bundle/Contents/Resources/licenses/native"
            native.mkdir(parents=True)
            (native / "dav1d-COPYING").write_text("Native copyright\n")
            (root / "bundle/versions.txt").write_text("Exact native versions\n")
            bundle, count = notices.generate(root, metadata, policy, native)
            self.assertEqual(count, 1)
            self.assertEqual(bundle["crates/sample-1.0.0/LICENSE-MIT"], b"Copyright Sample; MIT terms\n")
            self.assertEqual(bundle["native/versions.txt"], b"Exact native versions\n")
            self.assertNotIn("crates/sample-1.0.0/lib.rs", bundle)
            self.assertIn("fonts/OFL.txt", bundle)
            self.assertEqual(bundle, notices.generate(root, metadata, policy, native)[0])
            (crate / "LICENSE-MIT").unlink()
            (crate / "NOTICE").unlink()
            missing = notices.generate(root, metadata, policy, root / "absent")[0]
            text = missing["THIRD-PARTY-NOTICES.md"].decode()
            self.assertIn("MISSING: crate archive", text)
            self.assertIn("TODO: native notices are not yet staged", text)
            package["license"] = "LicenseRef-Fake-Disallowed"
            metadata_file = root / "metadata.json"
            metadata_file.write_text(json.dumps(metadata))
            output = root / "output"
            result = subprocess.run([sys.executable, str(SCRIPT), "--metadata", str(metadata_file),
                                     "--policy", str(policy), "--output", str(output)],
                                    capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("disallowed license for sample@1.0.0", result.stderr)
            self.assertFalse(output.exists())
            package["license"] = "Apache-2.0 WITH LLVM-exception"
            with self.assertRaises(ValueError):
                notices.generate(root, metadata, policy, native)
            policy.write_text('[licenses]\nallow = ["MIT"]\n[[licenses.exceptions]]\n'
                              'crate = "sample@1.0.0"\nallow = ["Apache-2.0 WITH LLVM-exception"]\n')
            self.assertEqual(notices.generate(root, metadata, policy, native)[1], 1)


if __name__ == "__main__":
    unittest.main()
