"""Tests for the local launcher; no Rust build or browser is needed."""
import functools
import http.client
import json
from pathlib import Path
import tempfile
import threading
import unittest
from unittest.mock import patch

from web_ui import Handler, LIMIT, ROOT, ThreadingHTTPServer, read_document


def setUpModule():
    (ROOT / "target").mkdir(exist_ok=True)


class DocumentTests(unittest.TestCase):
    def test_accepts_snapshots_and_arrays_and_paths_with_spaces(self):
        with tempfile.TemporaryDirectory(dir=ROOT / "target") as directory:
            path = Path(directory) / "decoded data.json"
            for document in ({"threads": []}, [{"threads": []}, {"threads": []}]):
                path.write_text(json.dumps(document), encoding="utf-8")
                self.assertEqual(json.loads(read_document(path)), document)

    def test_rejects_invalid_json_utf8_missing_and_oversized_files(self):
        with tempfile.TemporaryDirectory(dir=ROOT / "target") as directory:
            path = Path(directory) / "data.json"
            with self.assertRaises(FileNotFoundError):
                read_document(path)
            for data in (b"{", b"\xff", b" " * (LIMIT + 1)):
                path.write_bytes(data)
                with self.assertRaises(ValueError):
                    read_document(path)

    def test_limit_is_in_bytes_and_accepts_exact_limit(self):
        with tempfile.TemporaryDirectory(dir=ROOT / "target") as directory:
            path = Path(directory) / "data.json"
            path.write_bytes(b"[]" + b" " * (LIMIT - 2))
            self.assertEqual(len(read_document(path)), LIMIT)
            with path.open("ab") as file:
                file.write(b" ")
            with self.assertRaisesRegex(ValueError, "16 MiB"):
                read_document(path)


class QuietHandler(Handler):
    def log_message(self, *args):
        pass


class ServerTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(dir=ROOT / "target")
        self.addCleanup(self.directory.cleanup)
        site = Path(self.directory.name)
        (site / "index.html").write_text("<title>ZView</title>", encoding="utf-8")
        self.site_patch = patch("web_ui.SITE", site)
        self.site_patch.start()
        self.addCleanup(self.site_patch.stop)
        self.document = b'{"threads":[]}'
        handler = functools.partial(
            QuietHandler, document=self.document, name="decoded data.json"
        )
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
        self.thread = threading.Thread(target=self.server.serve_forever)
        self.thread.start()
        self.addCleanup(self.stop_server)

    def stop_server(self):
        self.server.shutdown()
        self.thread.join()
        self.server.server_close()

    def get(self, path, headers=None):
        connection = http.client.HTTPConnection("127.0.0.1", self.server.server_port)
        try:
            connection.request("GET", path, headers=headers or {})
            response = connection.getresponse()
            return response.status, dict(response.getheaders()), response.read()
        finally:
            connection.close()

    def test_serves_site_and_selected_document_without_copying_it(self):
        self.assertEqual(self.get("/")[0], 200)
        status, headers, data = self.get("/initial.json")
        self.assertEqual(status, 200)
        self.assertEqual(data, self.document)
        self.assertEqual(headers["Content-Type"], "application/json")
        self.assertEqual(headers["Cache-Control"], "no-store")
        self.assertNotIn("Access-Control-Allow-Origin", headers)
        session = json.loads(self.get("/session.json")[2])
        self.assertEqual(session, {"name": "decoded data.json"})
        self.assertFalse((Path(self.directory.name) / "initial.json").exists())

    def test_does_not_serve_repository_or_parent_paths_or_directory_lists(self):
        for path in ("/Cargo.toml", "/sample.json", "/../Cargo.toml", "/%2e%2e/Cargo.toml"):
            self.assertEqual(self.get(path)[0], 404)
        (Path(self.directory.name) / "fonts").mkdir()
        self.assertEqual(self.get("/fonts/")[0], 404)

    def test_rejects_foreign_hosts_and_cross_site_requests(self):
        self.assertEqual(self.get("/initial.json", {"Host": "example.com"})[0], 403)
        self.assertEqual(
            self.get("/initial.json", {"Sec-Fetch-Site": "cross-site"})[0], 403
        )


if __name__ == "__main__":
    unittest.main()
