"""引擎主动通知:与 RPC 响应共用 stdout 的事件帧。

事件帧遵循 IPC 契约的 event 信封(protocol/type/payload,无 id),宿主按
契约中的事件策略转发给前端。未绑定输出(一次性 rpc/测试)时静默丢弃。
"""
from __future__ import annotations

import json
import logging
import threading

from ..contracts.events import FERRY_EVENT_POLICIES
from ..contracts.ipc import FERRY_IPC_PROTOCOL

log = logging.getLogger(__name__)


class Notifier:
    def __init__(self):
        self._write = None
        self._lock = threading.Lock()

    def bind(self, write) -> None:
        """write 接收单行字符串并负责换行/flush/输出互斥。"""
        with self._lock:
            self._write = write

    def emit(self, event_type: str, payload: dict) -> None:
        policy = FERRY_EVENT_POLICIES.get(event_type)
        if policy is None or policy["source"] != "engine":
            raise ValueError(f"未注册的引擎事件: {event_type}")
        with self._lock:
            write = self._write
        if write is None:
            return
        frame = json.dumps(
            {
                "protocol": FERRY_IPC_PROTOCOL,
                "type": event_type,
                "payload": payload,
            },
            ensure_ascii=False,
        )
        try:
            write(frame)
        except Exception:  # noqa: BLE001 - 通知失败不能拖垮引擎主流程
            log.exception("引擎事件发送失败: %s", event_type)
