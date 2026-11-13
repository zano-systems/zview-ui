"""Build-first session context without a board, probe, or duplicated ELF decoder."""

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import yaml
from aiohttp.test_utils import AioHTTPTestCase
from zview.runner_config import BuildDirError

from build_context import checkout_context, resolve_build
from server import create_app

ROOT = Path(__file__).resolve().parent.parent
ELF = ROOT / "test-data/tests/fixtures/zephyr.elf"
RECORDING = ROOT / "test-data/tests/fixtures/recordings/sys_heap_v4.3.ndjson.gz"


def make_build(root, name="build", board="nrf5340dk/nrf5340/cpuapp"):
    build = root / name
    zephyr = build / "zephyr"
    zephyr.mkdir(parents=True)
    shutil.copyfile(ELF, zephyr / "zephyr.elf")
    (zephyr / ".config").write_text(f'CONFIG_BOARD_TARGET="{board}"\n')
    (zephyr / "runners.yaml").write_text(yaml.safe_dump({
        "flash-runner": "jlink",
        "debug-runner": "jlink",
        "runners": ["jlink", "pyocd"],
        "config": {"elf_file": "zephyr.elf"},
        "args": {
            "jlink": ["--device=nRF5340_xxAA", "--speed=4000"],
            "pyocd": ["--target=nrf5340_xxaa", "--frequency=2000000"],
        },
    }))
    source = root / "application"
    source.mkdir(exist_ok=True)
    (build / "CMakeCache.txt").write_text(
        f"APPLICATION_SOURCE_DIR:PATH={source}\nBOARD:STRING=cache-fallback\n"
    )
    return build


class BuildContextTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.build = make_build(self.root)

    def resolve(self, **changes):
        return resolve_build(self.root, {"mode": "live", "build_dir": "build", **changes})

    def test_live_resolves_effective_settings_without_opening_a_probe(self):
        with patch("build_context.Target.open") as opened:
            context = self.resolve()
            opened.assert_not_called()
        self.assertEqual(context["board"], "nrf5340dk/nrf5340/cpuapp")
        self.assertEqual(context["runner"], "jlink")
        self.assertEqual(context["target"], "nRF5340_xxAA")
        self.assertEqual(context["frequency"], 4000000)
        self.assertEqual(context["elf"], "build/zephyr/zephyr.elf")
        self.assertEqual(context["domain"], "app")
        self.assertEqual(context["project"], "application")
        self.assertIsNone(context["elf_override"])
        override = self.resolve(runner="pyocd", target="override", frequency=1000000)
        self.assertEqual((override["runner"], override["target"], override["frequency"]),
                         ("pyocd", "override", 1000000))

    def test_replay_needs_firmware_not_a_supported_probe_runner(self):
        (self.build / "zephyr/runners.yaml").write_text(yaml.safe_dump({
            "flash-runner": "unsupported", "config": {"elf_file": "zephyr.elf"},
        }))
        context = self.resolve(mode="replay")
        self.assertIsNone(context["runner"])
        self.assertEqual(context["elf"], "build/zephyr/zephyr.elf")
        override = self.resolve(mode="replay", elf="build/zephyr/zephyr.elf")
        self.assertEqual(override["elf_override"], override["elf"])

    def test_debug_runner_takes_precedence_over_the_flash_runner(self):
        runners = self.build / "zephyr/runners.yaml"
        data = yaml.safe_load(runners.read_text())
        data["flash-runner"] = "nrfutil"
        data["runners"] = ["nrfutil", "nrfjprog", "jlink", "pyocd"]
        data["args"]["jlink"] = ["--device=nRF52833_xxAA", "--speed=4000"]
        runners.write_text(yaml.safe_dump(data))
        with patch("build_context.Target.open") as opened:
            context = self.resolve()
            opened.assert_not_called()
        self.assertEqual((context["runner"], context["target"], context["frequency"]),
                         ("jlink", "nRF52833_xxAA", 4000000))
        self.assertEqual(context["elf"], "build/zephyr/zephyr.elf")
        self.assertEqual(context["warnings"], [])
        override = self.resolve(runner="pyocd")
        self.assertEqual((override["runner"], override["target"], override["frequency"]),
                         ("pyocd", "nrf5340_xxaa", 2000000))

    def test_unsupported_debug_runner_fallback_is_reported(self):
        runners = self.build / "zephyr/runners.yaml"
        data = yaml.safe_load(runners.read_text())
        data["debug-runner"] = "openocd"
        data["flash-runner"] = "unsupported"
        runners.write_text(yaml.safe_dump(data))
        context = self.resolve()
        self.assertEqual(context["runner"], "jlink")
        self.assertTrue(any("debug runner 'openocd'" in warning for warning in context["warnings"]))
        self.assertTrue(any("Using build-configured 'jlink'" in warning for warning in context["warnings"]))

    def test_supported_flash_runner_works_without_debug_metadata(self):
        runners = self.build / "zephyr/runners.yaml"
        data = yaml.safe_load(runners.read_text())
        del data["debug-runner"]
        data["flash-runner"] = "pyocd"
        runners.write_text(yaml.safe_dump(data))
        self.assertEqual(self.resolve()["runner"], "pyocd")

    def test_sysbuild_default_and_explicit_image_use_package_resolution(self):
        other = make_build(self.root, "network", board="nrf5340dk/nrf5340/cpunet")
        (self.root / "domains.yaml").write_text(yaml.safe_dump({
            "default": "application",
            "build_dir": str(self.root),
            "domains": [
                {"name": "application", "build_dir": str(self.build)},
                {"name": "network", "build_dir": str(other)},
            ],
        }))
        default = self.resolve(build_dir=".")
        self.assertEqual(default["domain"], "application")
        self.assertEqual(default["domains"], ["application", "network"])
        network = self.resolve(build_dir=".", domain="network")
        self.assertEqual(network["board"], "nrf5340dk/nrf5340/cpunet")
        self.assertEqual(network["elf"], "network/zephyr/zephyr.elf")
        with self.assertRaises(BuildDirError):
            self.resolve(build_dir=".", domain="missing")

    def test_absent_metadata_is_unknown_and_config_precedes_cache(self):
        self.assertEqual(self.resolve()["board"], "nrf5340dk/nrf5340/cpuapp")
        (self.build / "zephyr/.config").unlink()
        self.assertEqual(self.resolve()["board"], "cache-fallback")
        (self.build / "CMakeCache.txt").unlink()
        context = self.resolve()
        self.assertIsNone(context["board"])
        self.assertTrue(context["warnings"])
        self.assertIsNone(context["git"])

    def test_git_is_labelled_current_checkout_not_firmware_provenance(self):
        source = self.root / "application"
        with patch("build_context.subprocess.check_output",
                   side_effect=["abc123\n", "feature\n", " M app.c\n"]):
            path, git = checkout_context(self.root, str(source))
        self.assertEqual(path, "application")
        self.assertEqual(git["provenance"], "current_checkout")
        self.assertTrue(git["dirty"])
        with patch("build_context.subprocess.check_output",
                   side_effect=subprocess.TimeoutExpired("git", 2)):
            self.assertEqual(checkout_context(self.root, str(source)), ("application", None))
        self.assertEqual(checkout_context(self.root, str(self.root.parent)), (None, None))

    def test_invalid_overrides_and_root_escapes_are_rejected(self):
        for changes in (
            {"build_dir": ".."}, {"build_dir": "build/zephyr/zephyr.elf"},
            {"runner": "unknown"}, {"target": []}, {"frequency": True},
            {"frequency": -1}, {"frequency": "1000"}, {"domain": ""},
            {"mode": "other"}, {"mode": "replay", "runner": "jlink"},
            {"unexpected": True},
        ):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                self.resolve(**changes)
        with self.assertRaises(ValueError):
            resolve_build(self.root, [])

    @unittest.skipIf(os.name == "nt", "Symlinks can require Windows privileges.")
    def test_generated_artifact_and_domain_symlinks_cannot_escape_root(self):
        with tempfile.TemporaryDirectory() as outside:
            external = Path(outside) / "runners.yaml"
            external.write_text("{}")
            runners = self.build / "zephyr/runners.yaml"
            runners.unlink()
            runners.symlink_to(external)
            with self.assertRaisesRegex(ValueError, "data root"):
                self.resolve()
            runners.unlink()
            (self.root / "domains.yaml").write_text(yaml.safe_dump({
                "default": "outside", "build_dir": str(self.root),
                "domains": [{"name": "outside", "build_dir": outside}],
            }))
            with self.assertRaisesRegex(ValueError, "data root"):
                self.resolve(build_dir=".")


class BuildApiTests(AioHTTPTestCase):
    async def get_application(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        make_build(self.root)
        shutil.copyfile(RECORDING, self.root / "capture.ndjson.gz")
        return create_app(self.root)

    async def test_preview_is_read_only_and_errors_are_actionable(self):
        response = await self.client.post("/api/build/resolve", json={
            "mode": "live", "build_dir": "build",
        })
        self.assertEqual(response.status, 200)
        self.assertEqual((await response.json())["runner"], "jlink")
        self.assertIsNone((await (await self.client.get("/api/session")).json())["data"])
        for data in ({}, [], {"mode": "live", "build_dir": ".."},
                     {"mode": "live", "build_dir": "build", "domain": "missing"}):
            response = await self.client.post("/api/build/resolve", json=data)
            self.assertEqual(response.status, 400)
            self.assertTrue((await response.json())["error"])
        (self.root / "build/zephyr/runners.yaml").write_text("args: [")
        response = await self.client.post("/api/build/resolve", json={
            "mode": "live", "build_dir": "build",
        })
        self.assertEqual(response.status, 400)
        response = await self.client.post("/api/build/resolve", data="build")
        self.assertEqual(response.status, 415)
        response = await self.client.post("/api/build/resolve",
            json={"mode": "replay", "build_dir": "build"},
            headers={"Origin": "https://untrusted.example"})
        self.assertEqual(response.status, 403)

    async def test_build_context_survives_replay_and_snapshot_reconnect(self):
        response = await self.client.post("/api/session/replay", json={
            "recording": "capture.ndjson.gz", "build_dir": "build",
        })
        self.assertEqual(response.status, 202)
        opened = await response.json()
        context = opened["data"]["context"]
        self.assertEqual(context["board"], "nrf5340dk/nrf5340/cpuapp")
        self.assertEqual(opened["data"]["elf"], "build/zephyr/zephyr.elf")
        socket = await self.client.ws_connect("/api/events")
        self.assertEqual((await socket.receive_json())["data"]["context"], context)
        # Previewing another mode/build does not alter the current replay.
        await self.client.post("/api/build/resolve", json={"mode": "live", "build_dir": "build"})
        snapshot = await (await self.client.get("/api/session")).json()
        self.assertEqual(snapshot["session_id"], opened["session_id"])
        self.assertEqual(snapshot["data"]["context"], context)
        bad = await self.client.post("/api/session/replay", json={
            "recording": "capture.ndjson.gz", "build_dir": "../outside",
        })
        self.assertEqual(bad.status, 400)
        await self.client.post("/api/session/stop", json={"session_id": opened["session_id"]})
        await socket.close()


if __name__ == "__main__":
    unittest.main()
