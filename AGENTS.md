# Agent instructions

- Before claiming a UI feature done, screenshot it running against live
  providers, including any new degraded states. Use headless Chrome
  (`--headless --window-size=1280,900 --screenshot=PATH URL`), save outside
  the repo, inspect each, and attach them to the Fizzy card comment.
- Keep `README.md` short. Touch it only for important changes: a new or
  removed headline feature, configuration variable or run command. One
  line per feature; no routes, edge cases or behaviour details (those
  belong in `openapi.json`, the in-app help and the code). Most changes
  need no README edit.
