# Agent instructions

- Before claiming a UI feature done, screenshot it running against live
  providers, including any new degraded states. Use headless Chrome
  (`--headless --window-size=1280,900 --screenshot=PATH URL`), save outside
  the repo, inspect each, and attach them to the Fizzy card comment.
- Keep `README.md` current. When a change adds, removes or significantly
  changes a user-visible feature, route, configuration variable or
  degraded state, update the matching README bullets in the same commit.
  Keep it scannable: short bullets under feature headings, not prose.
