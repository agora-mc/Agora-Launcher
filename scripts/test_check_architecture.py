"""Hermetic fixtures for the game boundaries, even before Minecraft moves.

Run: python -m unittest discover -s scripts -p test_check_architecture.py
"""
import contextlib
import io
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import check_architecture as architecture


class GameBoundaryTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.core = self.root / "crates/agora-core/src"
        self.core.mkdir(parents=True)
        self.api = self.root / "crates/agora-game-api/Cargo.toml"
        self.api.parent.mkdir(parents=True)
        (self.root / "Cargo.toml").write_text(
            '[workspace.dependencies]\nserde = "1"\n', encoding="utf-8")
        (self.core.parent / "Cargo.toml").write_text('[dependencies]\n', encoding="utf-8")
        self.api.write_text('[dependencies]\nserde = { workspace = true }\nsemver = "1"\nthiserror = "1"\n', encoding="utf-8")
        for name, value in {
            "REPO_ROOT": self.root, "CORE_SRC": self.core,
            "CORE_CARGO": self.core.parent / "Cargo.toml", "GAME_API_CARGO": self.api,
            "EXIT_CODE": 0,
        }.items():
            patcher = patch.object(architecture, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)

    def check(self, function):
        architecture.EXIT_CODE = 0
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            function()
        return architecture.EXIT_CODE

    def test_allowed_contract_and_absent_minecraft_package(self):
        (self.core / "lib.rs").write_text('pub mod minecraft_runtime;\n', encoding="utf-8")
        self.assertEqual(self.check(architecture.check_game_api_dependencies), 0)
        self.assertEqual(self.check(architecture.check_core_no_minecraft_package), 0)

    def test_dependency_sections_aliases_and_workspace_resolution(self):
        for section in ["dependencies", "dev-dependencies", "build-dependencies", "target.'cfg(windows)'.dependencies"]:
            for dependency in ['tokio = "1"', 'alias = { package = "agora-core", path = "../agora-core" }']:
                with self.subTest(section=section, dependency=dependency):
                    self.api.write_text(f'[{section}]\n{dependency}\n', encoding="utf-8")
                    self.assertEqual(self.check(architecture.check_game_api_dependencies), 1)
        self.api.write_text('[dependencies]\nserde = { workspace = true }\n', encoding="utf-8")
        (self.root / "Cargo.toml").write_text('[workspace.dependencies]\nserde = { package = "tokio", version = "1" }\n', encoding="utf-8")
        self.assertEqual(self.check(architecture.check_game_api_dependencies), 1)

    def test_path_disguised_as_allowed_dependency_is_rejected(self):
        self.api.write_text('[dependencies]\nserde = { path = "../fake-serde" }\n', encoding="utf-8")
        self.assertEqual(self.check(architecture.check_game_api_dependencies), 1)

    def test_core_module_import_include_and_alias_dependency(self):
        for source in [
            'use agora_game_minecraft::launch;\n',
            'use agora_game_minecraft\n ::launch;\n',
            '#[path = "../../agora-game-minecraft/src/launch.rs"]\nmod launch;\n',
            'include!("../../agora-game-minecraft/src/launch.rs");\n',
            'extern crate agora_game_minecraft as minecraft;\n',
        ]:
            with self.subTest(source=source):
                (self.core / "lib.rs").write_text(source, encoding="utf-8")
                self.assertEqual(self.check(architecture.check_core_no_minecraft_package), 1)
        (self.core / "lib.rs").write_text('// agora_game_minecraft::launch\n/* agora-game-minecraft */\n', encoding="utf-8")
        self.assertEqual(self.check(architecture.check_core_no_minecraft_package), 0)
        (self.core.parent / "Cargo.toml").write_text(
            '[dependencies]\nmc = { package = "agora-game-minecraft", path = "../agora-game-minecraft" }\n', encoding="utf-8")
        self.assertEqual(self.check(architecture.check_core_no_minecraft_package), 1)

    def test_missing_and_invalid_contract_manifest_fail(self):
        self.api.unlink()
        self.assertEqual(self.check(architecture.check_game_api_dependencies), 1)
        self.api.write_text('invalid toml [', encoding="utf-8")
        self.assertEqual(self.check(architecture.check_game_api_dependencies), 1)

    def test_core_build_scripts_and_integration_tests_cannot_include_package_modules(self):
        for relative in ["build.rs", "tests/package_boundary.rs"]:
            with self.subTest(source=relative):
                source = self.core.parent / relative
                source.parent.mkdir(parents=True, exist_ok=True)
                source.write_text('use agora_game_minecraft::launch;\n', encoding="utf-8")
                self.assertEqual(self.check(architecture.check_core_no_minecraft_package), 1)
                source.unlink()


if __name__ == "__main__":
    unittest.main()
