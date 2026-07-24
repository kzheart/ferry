"""结构化事件：code + params，渲染语言由 UI 决定。"""
from __future__ import annotations


def event(code: str, severity: str = "warning", **params) -> dict:
    return {"code": code, "severity": severity, "params": params}
