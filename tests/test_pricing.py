"""models.dev 单价扁平化与缓存/兜底行为。"""

from engine.application import pricing as pr


def test_flatten_extracts_cost_per_model():
    api = {
        "anthropic": {"models": {
            "claude-opus-4-8": {"cost": {"input": 5, "output": 25,
                                         "cache_read": 0.5, "cache_write": 6.25}},
            "no-cost-model": {"id": "x"}}},
        "openai": {"models": {
            "gpt-5-codex": {"cost": {"input": 1.25, "output": 10}}}},
        "garbage": "not-a-dict",
    }
    flat = pr._flatten(api)
    assert flat["claude-opus-4-8"] == {"input": 5, "output": 25,
                                       "cache_read": 0.5, "cache_write": 6.25}
    # 缺省 cache 字段补 0
    assert flat["gpt-5-codex"] == {"input": 1.25, "output": 10,
                                   "cache_read": 0, "cache_write": 0}
    # 无 cost 的模型跳过
    assert "no-cost-model" not in flat


def test_pricing_uses_fresh_cache_without_network(tmp_path, monkeypatch):
    cache = tmp_path / "models-dev.json"
    monkeypatch.setattr(pr, "CACHE", cache)
    import json
    import time
    cache.write_text(json.dumps({
        "prices": {"claude-opus-4-8": {"input": 5, "output": 25,
                                       "cache_read": 0.5, "cache_write": 6.25}},
        "fetched_at": int(time.time() * 1000)}))

    def _boom():
        raise AssertionError("不应联网:缓存仍新鲜")
    monkeypatch.setattr(pr, "_fetch", _boom)

    result = pr.pricing()
    assert result["source"] == "cache"
    assert result["prices"]["claude-opus-4-8"]["input"] == 5


def test_pricing_falls_back_when_offline(tmp_path, monkeypatch):
    monkeypatch.setattr(pr, "CACHE", tmp_path / "missing.json")

    def _boom():
        raise OSError("offline")
    monkeypatch.setattr(pr, "_fetch", _boom)

    result = pr.pricing(force=True)
    assert result["source"] == "fallback"
    assert result["prices"]  # 内置兜底非空
