@AGENTS.md

## Claude Code specifics

- Opus is the main model. Delegate expensive work (bulk scaffolding, builds, wide research or verification sweeps) to Sonnet subagents; keep decisions and review on Opus. Use another model only if the user asks.
