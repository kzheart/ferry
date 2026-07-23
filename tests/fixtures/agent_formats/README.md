# Agent format fixtures

These fixtures are captured from real agent CLIs and are test-only contracts.
Production writers use the single current structure in
`engine/adapters/<agent>/native_schema.py`; the sidecar does not bundle this
directory.

## Refreshing an agent format

1. Capture a plain conversation and a conversation containing shell, write, and
   read tool calls under `<agent>/case-*/`.
2. Keep the native records unchanged apart from replacing private paths,
   identifiers, and message content with deterministic fixture values.
3. Extract the candidate production templates:

   ```bash
   python scripts/extract-agent-format.py <agent> \
     tests/fixtures/agent_formats/<agent>/case-02-tools/session.jsonl
   ```

   OpenCode captures use `session.json` instead of `session.jsonl`.
4. Compare the output with `engine/adapters/<agent>/native_schema.py`. If the
   required structure changed, replace the current templates, reader, writer,
   and fixtures together. Do not add a version branch.
5. Run
   `python -m pytest tests/test_current_native_formats.py tests/test_reply_editing.py`.

The extractor intentionally only prints a candidate. Updating a production
structure remains an explicit, reviewable code change. The previous structure
is retained by Git history rather than production compatibility code.
