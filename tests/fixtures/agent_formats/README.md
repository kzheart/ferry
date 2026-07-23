# Agent format fixtures

These fixtures are captured from real agent CLIs and are test-only contracts.
Production writers use declarative profiles in
`engine/adapters/<agent>/formats.py`; the sidecar does not bundle this directory.

## Refreshing an agent format

1. Capture a plain conversation and a conversation containing shell, write, and
   read tool calls under `<agent>/<cli-version>/`.
2. Keep the native records unchanged apart from replacing private paths,
   identifiers, and message content with deterministic fixture values.
3. Extract the candidate production templates:

   ```bash
   python scripts/extract-agent-format.py <agent> \
     tests/fixtures/agent_formats/<agent>/<version>/case-02-tools/session.jsonl
   ```

   OpenCode captures use `session.json` instead of `session.jsonl`.
4. Compare the output with `engine/adapters/<agent>/formats.py`. Add a new
   `FormatProfile` when the structure changed; otherwise add the captured CLI
   version to the existing profile's `tested_versions`.
5. Run `python -m pytest tests/test_format_profiles.py tests/test_authoring.py`.

The extractor intentionally only prints a candidate. Updating a production
profile remains an explicit, reviewable code change.
