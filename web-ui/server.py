"""Local-only HTTP/WebSocket adapter and static web frontend."""

import argparse
import asyncio
import contextlib
from pathlib import Path

from aiohttp import web
from yaml import YAMLError
from zview.runner_config import BuildDirError
from zview.target import Target

from build_context import resolve_build
from heap_capture import HeapCaptureBusy
from native_dialog import NativeDialogBusy, NativeDialogs, initial_directory, validate_selection
from paths import INPUT_LIMIT, input_path
from session import SessionBusy, SessionController
from view_selection import ViewBusy

HERE = Path(__file__).resolve().parent
FRONTEND = HERE.parent / "target" / "web"
CONTROLLER = web.AppKey("controller", SessionController)
DATA_ROOT = web.AppKey("data_root", Path)
SOCKETS = web.AppKey("sockets", set)
DIALOGS = web.AppKey("dialogs", NativeDialogs)


@web.middleware
async def local_only(request, handler):
    try:
        host = request.url.host
    except ValueError:
        raise web.HTTPForbidden(text="Invalid host.") from None
    if host not in {"localhost", "127.0.0.1", "::1"}:
        raise web.HTTPForbidden(text="Only loopback hosts are accepted.")
    origin = request.headers.get("Origin")
    if origin is not None and origin != f"{request.scheme}://{request.host}":
        raise web.HTTPForbidden(text="Cross-origin requests are not accepted.")
    if request.headers.get("Sec-Fetch-Site") == "cross-site":
        raise web.HTTPForbidden(text="Cross-site requests are not accepted.")
    response = await handler(request)
    response.headers["X-Content-Type-Options"] = "nosniff"
    response.headers["Cache-Control"] = "no-store"
    response.headers["Content-Security-Policy"] = (
        "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; "
        "style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data: blob:; "
        "frame-ancestors 'none'; base-uri 'none'; form-action 'self'"
    )
    return response


def json_response(message, status=200):
    return web.Response(
        text=message, content_type="application/json", status=status
    )


async def resolve_context(request):
    if request.content_type != "application/json":
        raise web.HTTPUnsupportedMediaType(text="Use application/json.")
    try:
        data = await request.json()
        context = await asyncio.to_thread(resolve_build, request.app[DATA_ROOT], data)
    except (ValueError, OSError, RuntimeError, KeyError, TypeError, BuildDirError, YAMLError) as exc:
        return web.json_response({"error": str(exc)}, status=400)
    return web.json_response(context)


async def get_session(request):
    return json_response(request.app[CONTROLLER].snapshot())


async def heap_detail(request):
    if request.content_type != "application/json":
        raise web.HTTPUnsupportedMediaType(text="Use application/json.")
    try:
        data = await request.json()
        if not isinstance(data, dict) or set(data) != {"session_id", "client_id", "heap"}:
            raise ValueError("Provide session_id, client_id and heap (or null to release).")
        if not all(isinstance(data[k], str) and data[k] for k in ("session_id", "client_id")):
            raise ValueError("Session and client identities must be nonempty strings.")
        heap = data["heap"]
        if heap is not None:
            if (
                not isinstance(heap, dict) or set(heap) != {"address", "name"}
                or type(heap["address"]) is not int or not 0 < heap["address"] < 2 ** 64
                or not isinstance(heap["name"], str) or not heap["name"]
            ):
                raise ValueError("Provide a valid heap address and name.")
            heap = heap["address"], heap["name"]
        controller = request.app[CONTROLLER]
        controller.choose_heap(data["session_id"], data["client_id"], heap)
    except ViewBusy as exc:
        return web.json_response({"error": str(exc), "code": "view_busy"}, status=409)
    except HeapCaptureBusy as exc:
        return web.json_response({"error": str(exc), "code": "heap_busy"}, status=409)
    except SessionBusy as exc:
        return web.json_response({"error": str(exc)}, status=409)
    except (ValueError, TypeError) as exc:
        return web.json_response({"error": str(exc)}, status=400)
    return web.json_response({"accepted": True})


async def select_view(request):
    if request.content_type != "application/json":
        raise web.HTTPUnsupportedMediaType(text="Use application/json.")
    try:
        data = await request.json()
        if (
            not isinstance(data, dict) or set(data) != {"session_id", "client_id", "view"}
            or not all(isinstance(data[k], str) and data[k] for k in ("session_id", "client_id"))
        ):
            raise ValueError("Provide session_id, client_id and view.")
        request.app[CONTROLLER].choose_view(data["session_id"], data["client_id"], data["view"])
    except ViewBusy as exc:
        return web.json_response({"error": str(exc), "code": "view_busy"}, status=409)
    except SessionBusy as exc:
        return web.json_response({"error": str(exc)}, status=409)
    except (ValueError, TypeError) as exc:
        return web.json_response({"error": str(exc)}, status=400)
    return web.json_response({"accepted": True})


async def choose_file(request):
    if request.content_type != "application/json":
        raise web.HTTPUnsupportedMediaType(text="Use application/json.")
    session = request.app[CONTROLLER].session
    project = session.context.get("source_dir") if session and session.context else None
    try:
        data = await request.json()
        if not isinstance(data, dict) or set(data) - {"kind", "current"}:
            raise ValueError("Provide a dialog kind and optional current path.")
        kind = data.get("kind")
        if not isinstance(kind, str) or kind not in {"directory", "elf", "recording"}:
            raise ValueError("Dialog kind must be directory, elf, or recording.")
        root = request.app[DATA_ROOT]
        initial = await asyncio.to_thread(initial_directory, root, data.get("current"), project)
        selected = await request.app[DIALOGS].choose(kind, initial)
        path = await asyncio.to_thread(validate_selection, root, selected, kind)
    except NativeDialogBusy as exc:
        return web.json_response({"error": str(exc)}, status=409)
    except (ValueError, OSError, RuntimeError) as exc:
        return web.json_response({"error": str(exc)}, status=400)
    return web.json_response({"path": path})


async def open_replay(request):
    if request.content_type != "application/json":
        raise web.HTTPUnsupportedMediaType(text="Use application/json.")
    try:
        data = await request.json()
        allowed = {"recording", "elf", "build_dir", "domain"}
        if not isinstance(data, dict) or set(data) - allowed or "recording" not in data:
            raise ValueError("Provide recording and ELF or a build directory.")
        root = request.app[DATA_ROOT]
        recording, recording_label = input_path(root, data["recording"])
        context = None
        if data.get("build_dir") is not None:
            context = await asyncio.to_thread(
                resolve_build, root, {"mode": "replay", **{
                    k: v for k, v in data.items() if k != "recording"
                }},
            )
            elf, elf_label = input_path(root, context["elf"])
        else:
            if data.get("domain") is not None:
                raise ValueError("Firmware image selection requires a build directory.")
            elf, elf_label = input_path(root, data.get("elf"))
    except (ValueError, OSError, RuntimeError, KeyError, TypeError, BuildDirError, YAMLError) as exc:
        return web.json_response({"error": str(exc)}, status=400)
    try:
        message = await request.app[CONTROLLER].start(
            recording, elf, recording_label, elf_label, context=context
        )
    except SessionBusy as exc:
        return web.json_response({"error": str(exc)}, status=409)
    return json_response(message, status=202)


async def stop_session(request):
    try:
        session_id = await command_session_id(request)
        return json_response(await request.app[CONTROLLER].stop(session_id))
    except ValueError as exc:
        return web.json_response({"error": str(exc)}, status=400)
    except SessionBusy as exc:
        return web.json_response({"error": str(exc)}, status=409)


async def command_session_id(request):
    if request.content_type != "application/json":
        raise web.HTTPUnsupportedMediaType(text="Use application/json.")
    data = await request.json()
    if (
        not isinstance(data, dict) or set(data) != {"session_id"}
        or not isinstance(data["session_id"], str) or not data["session_id"]
    ):
        raise ValueError("Provide the expected session_id.")
    return data["session_id"]


def live_configuration(root, data):
    allowed = {"build_dir", "domain", "elf", "runner", "target", "frequency"}
    if not isinstance(data, dict) or set(data) - allowed:
        raise ValueError("Provide a live build configuration.")
    context = resolve_build(root, {"mode": "live", **data})
    target = Target(
        elf=str(input_path(root, context["elf"])[0]),
        runner=context["runner"], target=context["target"], frequency=context["frequency"],
    )
    source = {
        "mode": "live", "configuration": data,
        "runner": context["runner"], "target": context["target"], "frequency": context["frequency"],
    }
    return target, source, context


async def open_live(request):
    if request.content_type != "application/json":
        raise web.HTTPUnsupportedMediaType(text="Use application/json.")
    try:
        data = await request.json()
        target, source, context = await asyncio.to_thread(
            live_configuration, request.app[DATA_ROOT], data,
        )
        message = await request.app[CONTROLLER].start_live(
            target, source, context["elf"], context,
        )
    except SessionBusy as exc:
        return web.json_response({"error": str(exc)}, status=409)
    except (ValueError, OSError, RuntimeError, KeyError, TypeError, BuildDirError, YAMLError) as exc:
        return web.json_response({"error": str(exc)}, status=400)
    return json_response(message, status=202)


async def reconnect_session(request):
    try:
        session_id = await command_session_id(request)
        controller = request.app[CONTROLLER]
        controller.require_session(session_id)
        if controller.session.mode != "live":
            raise ValueError("Reconnect is available only for Live sessions.")
        # Resolve again: builds can move or change while the probe is disconnected.
        target, source, context = await asyncio.to_thread(
            live_configuration, request.app[DATA_ROOT],
            controller.session.source["configuration"],
        )
        message = await controller.start_live(
            target, source, context["elf"], context, expected_id=session_id,
        )
    except SessionBusy as exc:
        return web.json_response({"error": str(exc)}, status=409)
    except (ValueError, OSError, RuntimeError, KeyError, TypeError, BuildDirError, YAMLError) as exc:
        return web.json_response({"error": str(exc)}, status=400)
    return json_response(message, status=202)


async def events(request):
    client_id = request.query.get("client_id")
    if client_id is not None and (
        len(client_id) > 80 or not client_id or not all(c.isascii() and (c.isalnum() or c == "-") for c in client_id)
    ):
        raise web.HTTPBadRequest(text="Invalid browser connection identity.")
    controller = request.app[CONTROLLER]
    try:
        subscription = controller.subscribe(client_id)
    except SessionBusy as exc:
        return web.json_response({"error": str(exc)}, status=409)
    ws = web.WebSocketResponse(heartbeat=20, max_msg_size=1024)
    # Validate before preparing: middleware runs before the upgrade too.
    try:
        await ws.prepare(request)
    except BaseException:
        controller.unsubscribe(subscription)
        raise
    request.app[SOCKETS].add(ws)

    async def write():
        while True:
            message = await subscription.get()
            if message is None:
                await ws.close(code=1013, message=b"Slow client; reconnect.")
                return
            await ws.send_str(message)

    writer = asyncio.create_task(write())
    try:
        async for _ in ws:
            # This endpoint is server-to-client. Commands use HTTP.
            await ws.close(code=1003, message=b"Use HTTP for session commands.")
    finally:
        controller.unsubscribe(subscription)
        request.app[SOCKETS].discard(ws)
        writer.cancel()
        with contextlib.suppress(asyncio.CancelledError, ConnectionError):
            await writer
    return ws


async def shutdown(app):
    await app[DIALOGS].close()
    await app[CONTROLLER].stop()
    await asyncio.gather(
        *(
            ws.close(code=1001, message=b"Server shutting down.")
            for ws in tuple(app[SOCKETS])
        )
    )


async def frontend_asset(request):
    if not (FRONTEND / "index.html").is_file():
        raise web.HTTPServiceUnavailable(
            text="Rust frontend is not built. Run: just web-setup && just web-build"
        )
    filename = request.match_info.get("asset", "index.html")
    path = (FRONTEND / filename).resolve()
    if (
        not path.is_relative_to(FRONTEND.resolve())
        or path.suffix not in {".html", ".js", ".mjs", ".wasm", ".css", ".ttf", ".txt"}
        or not path.is_file()
    ):
        raise web.HTTPNotFound()
    return web.FileResponse(path)


def create_app(data_root, controller=None, dialogs=None, *,
               period=0.1, heap_period=1., heap_cooldown=1.):
    app = web.Application(middlewares=[local_only], client_max_size=8192)
    app[DATA_ROOT] = Path(data_root).resolve(strict=True)
    app[CONTROLLER] = controller if controller is not None else SessionController(
        period=period, heap_period=heap_period, heap_cooldown=heap_cooldown,
    )
    app[SOCKETS] = set()
    app[DIALOGS] = dialogs if dialogs is not None else NativeDialogs()
    app.router.add_post("/api/files/choose", choose_file)
    app.router.add_post("/api/build/resolve", resolve_context)
    app.router.add_get("/api/session", get_session)
    app.router.add_post("/api/session/replay", open_replay)
    app.router.add_post("/api/session/live", open_live)
    app.router.add_post("/api/session/reconnect", reconnect_session)
    app.router.add_post("/api/session/stop", stop_session)
    app.router.add_post("/api/session/heap-detail", heap_detail)
    app.router.add_post("/api/session/view", select_view)
    app.router.add_get("/api/events", events)
    app.router.add_get("/", frontend_asset)
    app.router.add_get("/{asset:.*}", frontend_asset)
    app.on_shutdown.append(shutdown)
    return app


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--period", type=float, default=0.1,
                        help="Live polling period in seconds; 0 polls back-to-back (default: 0.1).")
    parser.add_argument("--heap-period", type=float, default=1.,
                        help="Minimum period between live heap capture starts, in seconds (default: 1).")
    parser.add_argument("--heap-cooldown", type=float, default=1.,
                        help="Minimum delay after a completed map; 0 allows fastest capture (default: 1).")
    parser.add_argument(
        "--data-root",
        type=Path,
        default=HERE.parent,
        help="Trusted input directory; paths outside it are rejected.",
    )
    args = parser.parse_args()
    if not args.data_root.is_dir():
        parser.error("--data-root must be an existing directory")
    try:
        app = create_app(args.data_root, period=args.period, heap_period=args.heap_period,
                         heap_cooldown=args.heap_cooldown)
    except ValueError as exc:
        parser.error(str(exc))
    web.run_app(app, host="127.0.0.1", port=args.port)


if __name__ == "__main__":
    main()
