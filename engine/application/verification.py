"""探针应用门面；具体 CLI 执行由 infrastructure 持有。"""

from .ports import current


class ProbeTimeout(RuntimeError):
    pass


def run_probe(tool, session_id, dirpath=None, model=None):
    try:
        return current().adapter(tool).verifier(session_id, dirpath, model)
    except Exception as error:
        if error.__class__.__name__ == "ProbeTimeout":
            raise ProbeTimeout(str(error)) from error
        raise
