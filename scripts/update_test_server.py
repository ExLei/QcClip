"""仅供 debug 包测试的本地 GitHub Releases 服务。"""

import argparse
import hashlib
import json
import sys
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.error import HTTPError
from urllib.parse import parse_qs, urlparse, unquote
from pathlib import Path
from tempfile import TemporaryDirectory
from urllib.request import urlopen


SCENARIOS = ("update", "current", "empty", "prerelease", "mixed", "paginated", "not-found", "rate-limit", "error", "invalid", "fallback", "download-error", "download-truncated", "download-corrupt")


def release(tag, *, draft=False, prerelease=False):
    return {
        "tag_name": tag,
        "name": f"QcClip {tag}",
        "draft": draft,
        "prerelease": prerelease,
        "html_url": f"https://github.com/mosheng1/QcClip/releases/tag/{tag}",
        "body": "本地测试版本，验证更新标识与发布页跳转。",
    }


class UpdateServer(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, address, scenario="update", version="v1.0.0-beta.4", delay=0, package_directory=None, chunk_delay=0):
        super().__init__(address, UpdateHandler)
        self.scenario = scenario
        self.version = version
        self.delay = delay
        self.package_directory = Path(package_directory).resolve() if package_directory else None
        self.chunk_delay = chunk_delay


class UpdateHandler(BaseHTTPRequestHandler):
    def do_GET(self):
        parsed = urlparse(self.path)
        query = parse_qs(parsed.query)
        if parsed.path == "/control":
            scenario = query.get("scenario", [self.server.scenario])[0]
            if scenario not in SCENARIOS:
                self.respond(400, {"message": "未知测试场景"})
                return
            self.server.scenario = scenario
            self.server.version = query.get("version", [self.server.version])[0]
            self.respond(200, {"scenario": scenario, "version": self.server.version})
            return
        mirror = parsed.path.startswith("/mirror/")
        route = parsed.path.removeprefix("/mirror") if mirror else parsed.path
        if route.startswith("/packages/"):
            self.serve_package(unquote(route.removeprefix("/packages/")), mirror)
            return
        if route.startswith("/releases/tags/"):
            tag = unquote(route.removeprefix("/releases/tags/"))
            if tag != self.server.version:
                self.respond(404, {"message": "镜像尚未同步此版本"})
            else:
                self.respond(200, self.attach_packages(release(tag), mirror))
            return
        if route != "/releases":
            self.respond(404, {"message": "未找到接口"})
            return
        if not mirror:
            time.sleep(self.server.delay)
        scenario = self.server.scenario
        if scenario in ("not-found", "rate-limit", "error") or (scenario == "fallback" and not mirror):
            self.respond({"not-found": 404, "rate-limit": 429}.get(scenario, 500), {"message": "模拟请求失败"})
            return
        if scenario == "invalid":
            self.respond(200, b"invalid-json")
            return
        if scenario == "empty":
            payload = []
        elif scenario == "current":
            payload = [release("v1.0.0-beta.1", prerelease=True)]
        elif scenario == "prerelease":
            payload = [release("v1.0.0-rc2", prerelease=True)]
        elif scenario == "mixed":
            payload = [
                release("v1.1.0-beta2", prerelease=True),
                release("v99.0.0", draft=True),
                release("invalid-tag"),
                release("v1.0.0"),
                release("v1.1.0-beta10", prerelease=True),
            ]
        elif scenario == "paginated":
            if query.get("page", ["1"])[0] == "1":
                next_url = f"http://127.0.0.1:{self.server.server_port}/releases?page=2"
                self.respond(200, [release("v1.0.0-beta.1", prerelease=True)], {"Link": f'<{next_url}>; rel="next"'})
                return
            payload = [release(self.server.version)]
        else:
            payload = [release(self.server.version)]
        self.respond(200, [self.attach_packages(item, mirror) for item in payload])

    def attach_packages(self, item, mirror):
        item["assets"] = []
        directory = self.server.package_directory
        if directory and directory.is_dir():
            prefix = "/mirror" if mirror else ""
            for path in sorted(directory.iterdir()):
                if path.is_file() and path.name.startswith("QcClip_") and path.name.endswith(("-setup.exe", "-portable.zip")):
                    with path.open("rb") as source:
                        digest = hashlib.file_digest(source, "sha256").hexdigest()
                    item["assets"].append({"name": path.name, "size": path.stat().st_size,
                        "digest": f"sha256:{digest}",
                        "browser_download_url": f"http://127.0.0.1:{self.server.server_port}{prefix}/packages/{path.name}"})
        return item

    def serve_package(self, name, mirror):
        directory = self.server.package_directory
        if not directory or Path(name).name != name or "\\" in name:
            self.respond(404, {"message": "未找到更新包"})
            return
        path = (directory / name).resolve()
        if path.parent != directory or not path.is_file():
            self.respond(404, {"message": "未找到更新包"})
            return
        if self.server.scenario == "download-error" and not mirror:
            self.respond(500, {"message": "模拟 GitHub 下载失败"})
            return
        size = path.stat().st_size
        self.send_response(200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(size))
        self.end_headers()
        sent = 0
        try:
            with path.open("rb") as source:
                while block := source.read(64 * 1024):
                    if not mirror and self.server.scenario == "download-corrupt" and sent == 0:
                        block = bytes([block[0] ^ 255]) + block[1:]
                    self.wfile.write(block)
                    self.wfile.flush()
                    sent += len(block)
                    if not mirror and self.server.scenario == "download-truncated" and sent >= size // 2:
                        self.close_connection = True
                        return
                    time.sleep(self.server.chunk_delay)
        except (BrokenPipeError, ConnectionResetError):
            pass

    def respond(self, status, payload, headers=None):
        body = payload if isinstance(payload, bytes) else json.dumps(payload, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        for name, value in (headers or {}).items():
            self.send_header(name, value)
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, fmt, *args):
        print(f"更新测试请求：{fmt % args}", flush=True)


class ServerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = UpdateServer(("127.0.0.1", 0))
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        cls.base = f"http://127.0.0.1:{cls.server.server_port}"

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()
        cls.thread.join()

    def test_scenarios(self):
        for scenario in SCENARIOS:
            with self.subTest(scenario=scenario):
                with urlopen(f"{self.base}/control?scenario={scenario}") as response:
                    self.assertEqual(json.load(response)["scenario"], scenario)
                if scenario in ("not-found", "rate-limit", "error", "fallback"):
                    with self.assertRaises(HTTPError) as error:
                        urlopen(f"{self.base}/releases")
                    self.assertEqual(error.exception.code, {"not-found": 404, "rate-limit": 429, "error": 500, "fallback": 500}[scenario])
                    continue
                with urlopen(f"{self.base}/releases?page=1") as response:
                    if scenario == "invalid":
                        self.assertEqual(response.read(), b"invalid-json")
                        continue
                    payload = json.load(response)
                    self.assertIsInstance(payload, list)
                    if scenario == "empty":
                        self.assertEqual(payload, [])
                    if scenario == "mixed":
                        self.assertEqual(payload[-1]["tag_name"], "v1.1.0-beta10")
                    if scenario == "prerelease":
                        self.assertTrue(payload[0]["prerelease"])
                    if scenario == "paginated":
                        self.assertIn('rel="next"', response.headers["Link"])
                        with urlopen(f"{self.base}/releases?page=2") as second:
                            self.assertEqual(json.load(second)[0]["tag_name"], "v1.0.0-beta.4")

    def test_package_failure_and_mirror_routes(self):
        with TemporaryDirectory() as temporary:
            directory = Path(temporary)
            name = "QcClip_1.0.0-beta.4_x64-setup.exe"
            content = b"MZ-test-only"
            (directory / name).write_bytes(content)
            self.server.package_directory = directory
            self.server.version = "v1.0.0-beta.4"
            self.server.scenario = "download-error"
            try:
                with urlopen(f"{self.base}/releases") as response:
                    asset = json.load(response)[0]["assets"][0]
                    self.assertEqual(asset["digest"], "sha256:" + hashlib.sha256(content).hexdigest())
                with self.assertRaises(HTTPError):
                    urlopen(asset["browser_download_url"])
                with urlopen(f"{self.base}/mirror/releases/tags/v1.0.0-beta.4") as response:
                    mirror = json.load(response)["assets"][0]
                with urlopen(mirror["browser_download_url"]) as response:
                    self.assertEqual(response.read(), content)
            finally:
                self.server.package_directory = None
                self.server.scenario = "update"



def main():
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser(description="QcClip 本地更新检查测试服务，仅绑定本机地址")
    parser.add_argument("--port", type=int, default=18765, help="监听端口")
    parser.add_argument("--scenario", choices=SCENARIOS, default="update", help="测试场景")
    parser.add_argument("--version", default="v1.0.0-beta.4", help="模拟发布标签，可含测试版后缀")
    parser.add_argument("--delay", type=float, default=0, help="模拟响应延迟秒数")
    parser.add_argument("--package-directory", type=Path, help="提供真实更新包的目录，不会生成或执行安装包")
    parser.add_argument("--chunk-delay", type=float, default=0, help="每 64KB 下载延迟秒数")
    parser.add_argument("--self-test", action="store_true", help="运行服务自检")
    args = parser.parse_args()
    if args.self_test:
        unittest.main(argv=[__file__])
        return
    if args.delay < 0 or args.chunk_delay < 0:
        parser.error("响应延迟不能小于零")
    server = UpdateServer(("127.0.0.1", args.port), args.scenario, args.version, args.delay, args.package_directory, args.chunk_delay)
    print(f"更新测试服务：http://127.0.0.1:{server.server_port}/releases", flush=True)
    print(f"测试场景：{args.scenario}；模拟版本：{args.version}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("更新测试服务已停止", flush=True)
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
