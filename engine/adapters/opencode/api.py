"""受控启动 OpenCode 官方 server，并调用会话编辑 API。"""
from __future__ import annotations

import base64
import json
import os
import queue
import re
import secrets
import subprocess
import threading
import time
import urllib.error
import urllib.parse
import urllib.request

from ...infrastructure import executables


class OpenCodeApiError(RuntimeError):
    pass


class OpenCodeApi:
    SUPPORTED_VERSION = "1.18.4"

    def __init__(self, cwd: str, timeout: float = 20):
        self.cwd = cwd
        self.timeout = timeout
        self.process = None
        self.base_url = None
        self.username = "ferry"
        self.password = secrets.token_hex(32)
        self.version = None

    def __enter__(self):
        env = dict(os.environ)
        env.update({
            "OPENCODE_SERVER_USERNAME": self.username,
            "OPENCODE_SERVER_PASSWORD": self.password,
        })
        self.process = subprocess.Popen(
            executables.argv("opencode", "serve", "--pure",
                             "--hostname", "127.0.0.1", "--port", "0"),
            cwd=self.cwd, env=env, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, text=True, bufsize=1,
            **executables.RUN_FLAGS)
        lines = queue.Queue()
        def read_lines():
            for line in self.process.stdout:
                lines.put(line.rstrip())
        threading.Thread(target=read_lines, daemon=True).start()
        deadline = time.monotonic() + self.timeout
        output = []
        pattern = re.compile(r"opencode server listening on (https?://\S+)")
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                break
            try:
                line = lines.get(timeout=.1)
            except queue.Empty:
                continue
            output.append(line)
            match = pattern.search(line)
            if match:
                self.base_url = match.group(1).rstrip("/")
                break
        if not self.base_url:
            self.close()
            raise OpenCodeApiError("OpenCode server 启动失败: " + "\n".join(output[-10:]))
        health = self.request("GET", "/global/health")
        if not health.get("healthy"):
            self.close()
            raise OpenCodeApiError("OpenCode server 健康检查失败")
        self.version = str(health.get("version") or "")
        if self.version != self.SUPPORTED_VERSION:
            self.close()
            raise OpenCodeApiError(
                f"OpenCode {self.version or '未知版本'} 尚未通过原地编辑验证，"
                f"当前仅支持 {self.SUPPORTED_VERSION}")
        return self

    def close(self):
        if self.process and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)

    def __exit__(self, exc_type, exc, tb):
        self.close()

    def request(self, method: str, path: str, body=None):
        token = base64.b64encode(f"{self.username}:{self.password}".encode()).decode()
        url = self.base_url + path
        data = None if body is None else json.dumps(body, ensure_ascii=False).encode()
        headers = {"Authorization": f"Basic {token}"}
        if data is not None:
            headers["Content-Type"] = "application/json"
        request = urllib.request.Request(url, data=data, headers=headers, method=method)
        try:
            with urllib.request.urlopen(request, timeout=self.timeout) as response:
                raw = response.read()
                return json.loads(raw) if raw else None
        except urllib.error.HTTPError as error:
            detail = error.read().decode(errors="replace")[-500:]
            raise OpenCodeApiError(f"OpenCode API {method} {path} 返回 {error.code}: {detail}") from error
        except urllib.error.URLError as error:
            raise OpenCodeApiError(f"OpenCode API 连接失败: {error}") from error

    def _path(self, path: str) -> str:
        return path + "?" + urllib.parse.urlencode({"directory": self.cwd})

    def capabilities(self) -> dict:
        doc = self.request("GET", "/doc")
        paths = doc.get("paths", {}) if isinstance(doc, dict) else {}
        part_route = paths.get(
            "/session/{sessionID}/message/{messageID}/part/{partID}", {})
        message_route = paths.get(
            "/session/{sessionID}/message/{messageID}", {})
        batch_route = paths.get("/session/{sessionID}/edit", {})
        return {
            "patch_part": "patch" in part_route,
            "delete_message": "delete" in message_route,
            "batch_edit": "post" in batch_route,
        }

    def patch_part(self, session_id: str, message_id: str, part: dict):
        part_id = part["id"]
        path = f"/session/{session_id}/message/{message_id}/part/{part_id}"
        return self.request("PATCH", self._path(path), part)

    def assert_idle(self, session_id: str) -> None:
        statuses = self.request("GET", self._path("/session/status"))
        if isinstance(statuses, dict) and session_id in statuses:
            raise OpenCodeApiError(f"OpenCode 会话 {session_id} 正在运行，拒绝原地编辑")
