# MCP Tools
<!-- trace:v1 id=doc.tracelayer.mcp -->

Use TraceLayer through MCP when the harness speaks tools instead of shells.
Start the server with `trace mcp` (stdio). Eleven tools, one per question:

| Tool | Question it answers | CLI twin |
|---|---|---|
| `orient` | Where am I, what is active, what to read? | `trace orient` |
| `brief` | Bounded engineering briefing for this task? | `trace brief <query>` |
| `next` | What should I work on next? | `trace next` |
| `delta` | What did my edits affect? | `trace delta` |
| `status` | Is this repo traced and healthy? | `trace status` |
| `search` | What traces exist about X? | `trace search <query>` |
| `context` | What must I know before touching this? | `trace context <id>` |
| `why` | Why does this node exist (causal path to root)? | `trace why <id>` |
| `impact` | What breaks if I change this? | `trace impact <id>` |
| `verify` | Does the change pass policy? | `trace verify --changed` |
| `index` | Re-index first, then ask again. | `trace index --changed` |

Session-opening sequence for any edit task: `orient` → `brief <task>` →
targeted source reads. Pre-edit on shared behavior: `impact <id>`.
Pre-completion: `verify`. `context` records a context-load (the same signal
the pre-edit gate checks); loading context through MCP counts exactly like
the CLI.

Workflows in [workflows.md](workflows.md) name the CLI form; substitute the
MCP twin one-for-one when shelling out is unavailable.
