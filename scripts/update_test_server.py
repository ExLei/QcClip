"""仅供 debug 包测试的本地 GitHub Releases 服务。"""

import argparse
import json
import sys
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.error import HTTPError
from urllib.parse import parse_qs, urlparse
from urllib.request import urlopen


SCENARIOS = ("update", "current", "empty", "prerelease", "mixed", "paginated", "not-found", "rate-limit", "error", "invalid")


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

    def __init__(self, address, scenario="update", version="v0.1.2", delay=0):
        super().__init__(address, UpdateHandler)
        self.scenario = scenario
        self.version = version
        self.delay = delay


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
        if parsed.path != "/releases":
            self.respond(404, {"message": "未找到接口"})
            return
        time.sleep(self.server.delay)
        scenario = self.server.scenario
        if scenario in ("not-found", "rate-limit", "error"):
            self.respond({"not-found": 404, "rate-limit": 429, "error": 500}[scenario], {"message": "模拟请求失败"})
            return
        if scenario == "invalid":
            self.respond(200, b"invalid-json")
            return
        if scenario == "empty":
            payload = []
        elif scenario == "current":
            payload = [release("v0.1.0")]
        elif scenario == "prerelease":
            payload = [release("v0.1.2-rc2", prerelease=True)]
        elif scenario == "mixed":
            payload = [
                release("v0.2.0-beta2", prerelease=True),
                release("v99.0.0", draft=True),
                release("invalid-tag"),
                release("v0.1.2"),
                release("v0.2.0-beta10", prerelease=True),
            ]
        elif scenario == "paginated":
            if query.get("page", ["1"])[0] == "1":
                next_url = f"http://127.0.0.1:{self.server.server_port}/releases?page=2"
                self.respond(200, [release("v0.1.0")], {"Link": f'<{next_url}>; rel="next"'})
                return
            payload = [release(self.server.version)]
        else:
            payload = [release(self.server.version)]
        self.respond(200, payload)

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
                if scenario in ("not-found", "rate-limit", "error"):
                    with self.assertRaises(HTTPError) as error:
                        urlopen(f"{self.base}/releases")
                    self.assertEqual(error.exception.code, {"not-found": 404, "rate-limit": 429, "error": 500}[scenario])
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
                        self.assertEqual(payload[-1]["tag_name"], "v0.2.0-beta10")
                    if scenario == "prerelease":
                        self.assertTrue(payload[0]["prerelease"])
                    if scenario == "paginated":
                        self.assertIn('rel="next"', response.headers["Link"])
                        with urlopen(f"{self.base}/releases?page=2") as second:
                            self.assertEqual(json.load(second)[0]["tag_name"], "v0.1.2")


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser(description="QcClip 本地更新检查测试服务，仅绑定本机地址")
    parser.add_argument("--port", type=int, default=18765, help="监听端口")
    parser.add_argument("--scenario", choices=SCENARIOS, default="update", help="测试场景")
    parser.add_argument("--version", default="v0.1.2", help="模拟发布标签，可含测试版后缀")
    parser.add_argument("--delay", type=float, default=0, help="模拟响应延迟秒数")
    parser.add_argument("--self-test", action="store_true", help="运行服务自检")
    args = parser.parse_args()
    if args.self_test:
        unittest.main(argv=[__file__])
        return
    if args.delay < 0:
        parser.error("响应延迟不能小于零")
    server = UpdateServer(("127.0.0.1", args.port), args.scenario, args.version, args.delay)
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
