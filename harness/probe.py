#!/usr/bin/env python3
"""外部验证器 CLI；生产实现由 engine.operations.verification 所有。"""

import argparse
import sys

from engine.bootstrap import create_context
from engine.contracts.agents import AGENT_IDS
from engine.operations.verification import ProbeTimeout, run_probe


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument("tool", choices=AGENT_IDS)
    parser.add_argument("session_id")
    parser.add_argument("--dir", default=None)
    parser.add_argument("--model", default=None)
    args = parser.parse_args(argv)
    try:
        ok, detail = run_probe(
            args.tool,
            args.session_id,
            args.dir,
            args.model,
            ports=create_context(),
        )
    except ProbeTimeout as error:
        print(str(error), file=sys.stderr)
        return 3
    print(f"[{'PASS' if ok else 'FAIL'}] {args.tool} {args.session_id}\n{detail}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
