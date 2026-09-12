"""Regression coverage for tag stamping and Cargo's locked resolution.

Run from the repository root: python -m unittest discover -s scripts -p test_set_release_version.py
The Cargo integration check uses the local dependency cache, without compiling.
"""

import contextlib
import io
import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import set_release_version as release


class ReleaseVersionTests(unittest.TestCase):
    def test_stamped_workspace_resolves_without_lock_changes(self):
        metadata = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1", "--offline"],
            cwd=release.ROOT, text=True,
        ))
        members = [p for p in metadata["packages"] if p["id"] in metadata["workspace_members"]]
        self.assertEqual(set(release.WORKSPACE_MEMBERS), {p["name"] for p in members})

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            files = ["Cargo.toml", "Cargo.lock", "desktop/package.json",
                     "desktop/src-tauri/tauri.conf.json"]
            files += [str(Path(p["manifest_path"]).relative_to(release.ROOT)) for p in members]
            for filename in files:
                destination = root / filename
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(release.ROOT / filename, destination)
            # Metadata needs target paths, but does not compile their contents.
            for package in members:
                for target in package["targets"]:
                    path = root / Path(target["src_path"]).relative_to(release.ROOT)
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.touch()

            with patch.object(release, "ROOT", root), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(release.cmd_set("9.8.7"), 0)
                self.assertEqual(release.cmd_check(), 0)
                lock_before = (root / "Cargo.lock").read_bytes()
                subprocess.run(
                    ["cargo", "metadata", "--locked", "--offline", "--format-version", "1"],
                    cwd=root, stdout=subprocess.DEVNULL, check=True,
                )
                self.assertEqual((root / "Cargo.lock").read_bytes(), lock_before)
                # --check must reject a stale plugin entry as well.
                text = release.read(root / "Cargo.lock")
                text = release._lock_pattern("agora-plugin-host").sub(
                    r"\g<1>0.1.0\g<3>", text, count=1,
                )
                release.write(root / "Cargo.lock", text)
                self.assertEqual(release.cmd_check(), 1)


if __name__ == "__main__":
    unittest.main()
