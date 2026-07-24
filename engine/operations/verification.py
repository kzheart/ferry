"""探针应用门面；具体 CLI 执行由 infrastructure 持有。"""

from ..context import EngineContext


class ProbeTimeout(RuntimeError):
    pass


def timeout_report(tool, error) -> dict:
    return {"status": "failed", "code": "probe.timeout",
            "params": {"tool": tool},
            "diagnostic": {"stdout": "", "stderr": str(error),
                           "truncated": False}}


def run_probe(tool, session_id, dirpath=None, model=None, *,
              ports: EngineContext):
    try:
        return ports.adapter(tool).verifier.probe(
            session_id, dirpath, model)
    except Exception as error:
        if error.__class__.__name__ == "ProbeTimeout":
            raise ProbeTimeout(str(error)) from error
        raise
