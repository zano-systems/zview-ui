# SPDX-License-Identifier: Apache-2.0

"""``west zview-ui``: open the browser UI connected Live to the current build."""

import argparse
import asyncio
import os
import sys
from pathlib import Path

from west.commands import WestCommand

ROOT = Path(__file__).resolve().parents[2]
WEB_UI = ROOT / "web-ui"
SCRIPTS = ROOT / "scripts"

EPILOG = """\
Auto-detected when omitted:
  -d/--build-dir      build.dir-fmt, the current directory, or ./build
  -e/--elf-file       from the build directory's runners.yaml
  -r/--runner         the build's debug-runner, then flash-runner
  -t/--runner-target  from the build directory's runners.yaml
  --data-root         the west workspace, if it holds the build and its sources

One-time setup: `west packages pip --install`, then `just web-setup`
in the zview-ui module directory (nightly Rust + wasm-bindgen).
"""


class ZViewUiCommand(WestCommand):
    def __init__(self):
        super().__init__(
            "zview-ui",
            "open the ZView browser UI connected to the current build",
            "Build the ZView UI's WASM site, start its local adapter, connect Live "
            "to the target described by the build directory, and open the browser.",
        )

    def do_add_parser(self, parser_adder):
        parser = parser_adder.add_parser(
            self.name,
            help=self.help,
            description=self.description,
            epilog=EPILOG,
            formatter_class=argparse.RawDescriptionHelpFormatter,
        )
        parser.add_argument("-d", "--build-dir", help="Zephyr build directory.")
        parser.add_argument("--domain", help="Firmware image of a sysbuild build.")
        parser.add_argument("-e", "--elf-file", help="ELF override.")
        parser.add_argument("-r", "--runner", help="Probe runner override (gdb, jlink, nrfutil, pyocd).")
        parser.add_argument("-t", "--runner-target", help="MCU/target override.")
        parser.add_argument("--frequency", type=int, help="Probe clock override, in Hz.")
        parser.add_argument("--period", type=float, default=0.1,
                            help="Live polling period in seconds; 0 polls back-to-back (default: 0.1).")
        parser.add_argument("--heap-period", type=float, default=1.0,
                            help="Minimum period between heap map captures, in seconds (default: 1).")
        parser.add_argument("--heap-cooldown", type=float, default=1.0,
                            help="Minimum delay after a completed heap map, in seconds (default: 1).")
        parser.add_argument("--port", type=int,
                            help="Local port; 0 picks a free one (default: 8765, "
                                 "or a free port when 8765 is taken).")
        parser.add_argument("--data-root", help="Trusted directory the UI may read paths from.")
        parser.add_argument("--no-build", action="store_true",
                            help="Serve the existing WASM site without rebuilding it.")
        parser.add_argument("--no-open", action="store_true",
                            help="Do not open the browser.")
        return parser

    def do_run(self, args, unknown):
        del unknown
        build_dir = self._build_dir(args)
        self._use_zview()
        try:
            import aiohttp  # noqa: F401
        except ImportError:
            self.die("aiohttp is missing. Run `west packages pip --install`.")

        sys.path.insert(0, str(WEB_UI))
        import west_launch

        extra = [args.elf_file] if args.elf_file else []
        try:
            root = (Path(args.data_root).resolve() if args.data_root
                    else west_launch.data_root_for(build_dir, self.topdir, extra))
            live = west_launch.live_request(
                root, build_dir, domain=args.domain, elf=args.elf_file, runner=args.runner,
                target=args.runner_target, frequency=args.frequency,
            )
        except ValueError as error:
            self.die(f"{error} Pass --data-root containing the build and ELF.")
        self.inf(f"Build directory: {build_dir}")
        self.inf(f"Data root: {root}")

        if not args.no_build:
            self._build_site()

        import server

        try:
            asyncio.run(server.serve(
                root, port=args.port, period=args.period, heap_period=args.heap_period,
                heap_cooldown=args.heap_cooldown, live=live, open_browser=not args.no_open,
            ))
        except KeyboardInterrupt:
            pass
        except (OSError, ValueError) as error:
            self.die(str(error))

    def _build_dir(self, args):
        zephyr_base = os.environ.get("ZEPHYR_BASE")
        if zephyr_base is None:
            self.die("ZEPHYR_BASE is not set. Run inside a Zephyr west workspace.")
        sys.path.append(str(Path(zephyr_base) / "scripts" / "west_commands"))
        from build_helpers import find_build_dir, is_zephyr_build

        build_dir = find_build_dir(args.build_dir)
        if not is_zephyr_build(build_dir):
            self.die(f"'{build_dir}' is not a Zephyr build directory. "
                     "Run `west build` or pass -d.")
        return Path(build_dir).resolve()

    def _use_zview(self):
        """Prefer the manifest's zview checkout, then an installed package."""
        sys.path.insert(0, str(WEB_UI))
        from west_launch import zview_pin

        pin = zview_pin()
        try:
            project = self.manifest.get_projects(["zview"])[0]
        except ValueError:
            project = None
        if project is not None and project.is_cloned():
            sys.path.insert(0, str(Path(project.abspath) / "src"))
            head = project.sha("HEAD")
            if not head.startswith(pin):
                self.wrn(f"Manifest zview is at {head[:12]}; zview-ui is tested with {pin[:12]}.")
        try:
            import zview.core  # noqa: F401
            import zview.target  # noqa: F401
        except ImportError as error:
            self.die(f"ZView session API not found ({error}). Pin the manifest `zview` "
                     f"project to {pin}, or install that revision with pip.")

    def _build_site(self):
        sys.path.insert(0, str(SCRIPTS))
        import subprocess

        import web_ui

        try:
            web_ui.build()
        except ValueError as error:
            self.die(f"{error} (run it in {ROOT})")
        except (OSError, subprocess.CalledProcessError) as error:
            self.die(f"WASM build failed: {error}")
