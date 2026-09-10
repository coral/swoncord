"""Exercise release guards without making GitHub requests."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


SCRIPTS = Path(__file__).resolve().parent


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "Cargo.toml").write_text('[package]\nversion = "1.2.3"\n')
        scripts = self.root / "scripts/ci"
        scripts.mkdir(parents=True)
        for name in ("version.py", "publish-release.sh"):
            shutil.copy2(SCRIPTS / name, scripts / name)
        self.dist = self.root / "target/ci/dist"
        self.dist.mkdir(parents=True)
        self.asset = self.dist / "Swoncord-1.2.3-macos-arm64.dmg"
        self.asset.write_bytes(b"test disk image")
        self.calls = self.root / "calls.jsonl"
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        gh = bin_dir / "gh"
        gh.write_text('''#!/usr/bin/env python3
import json, os, sys
args = sys.argv[1:]
with open(os.environ["GH_CALLS"], "a") as output:
    output.write(json.dumps(args) + "\\n")
if args[1] == "view":
    mode = os.environ.get("GH_MODE", "new")
    if mode == "new":
        sys.exit(1)
    if args[args.index("--json") + 1] == "isDraft":
        print("false" if mode == "public" else "true")
    else:
        print(os.environ.get("GH_ASSETS", ""))
if args[1] == "upload" and os.environ.get("FAIL_UPLOAD") == "1":
    sys.exit(1)
''')
        gh.chmod(0o755)
        self.env = {
            **os.environ,
            "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
            "GITHUB_REF": "refs/tags/v1.2.3",
            "GH_CALLS": str(self.calls),
            "GH_MODE": "new",
            "GH_ASSETS": "",
            "FAIL_UPLOAD": "0",
        }

    def publish(self):
        return subprocess.run(
            ["bash", "scripts/ci/publish-release.sh"],
            cwd=self.root, env=self.env, capture_output=True, text=True, check=False,
        )

    def recorded_calls(self):
        return [json.loads(line) for line in self.calls.read_text().splitlines()]

    def test_mismatched_tag_cannot_publish(self):
        self.env["GITHUB_REF"] = "refs/tags/v9.9.9"
        self.assertNotEqual(self.publish().returncode, 0)
        self.assertFalse(self.calls.exists())

    def test_branch_build_cannot_publish(self):
        self.env["GITHUB_REF"] = "refs/heads/master"
        self.assertNotEqual(self.publish().returncode, 0)
        self.assertFalse(self.calls.exists())

    def test_missing_asset_cannot_publish(self):
        self.asset.unlink()
        self.assertNotEqual(self.publish().returncode, 0)
        self.assertFalse(self.calls.exists())

    def test_extra_asset_cannot_publish(self):
        (self.dist / "unsigned.dmg").write_bytes(b"unexpected")
        self.assertNotEqual(self.publish().returncode, 0)
        self.assertFalse(self.calls.exists())

    def test_only_expected_dmg_is_uploaded_before_publishing(self):
        result = self.publish()
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.recorded_calls()
        self.assertEqual([call[1] for call in calls], ["view", "create", "upload", "edit"])
        self.assertIn("--draft", calls[1])
        self.assertEqual(calls[2], ["release", "upload", "v1.2.3", str(self.asset.relative_to(self.root)), "--clobber"])
        self.assertIn("--draft=false", calls[3])

    def test_failed_upload_leaves_release_draft(self):
        self.env["FAIL_UPLOAD"] = "1"
        self.assertNotEqual(self.publish().returncode, 0)
        self.assertNotIn("edit", [call[1] for call in self.recorded_calls()])

    def test_public_release_cannot_be_overwritten(self):
        self.env["GH_MODE"] = "public"
        self.assertNotEqual(self.publish().returncode, 0)
        self.assertEqual([call[1] for call in self.recorded_calls()], ["view"])

    def test_draft_with_other_assets_cannot_be_published(self):
        self.env.update(GH_MODE="draft", GH_ASSETS="unsigned.dmg")
        self.assertNotEqual(self.publish().returncode, 0)
        self.assertEqual([call[1] for call in self.recorded_calls()], ["view", "view"])

    def test_interrupted_upload_can_be_retried(self):
        self.env.update(GH_MODE="draft", GH_ASSETS=self.asset.name)
        self.assertEqual(self.publish().returncode, 0)
        self.assertEqual([call[1] for call in self.recorded_calls()], ["view", "view", "upload", "edit"])


if __name__ == "__main__":
    unittest.main()
