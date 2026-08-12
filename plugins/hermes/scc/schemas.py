"""Tool schemas — what the LLM reads to decide when to call SCC tools."""

SYSTEM_OVERVIEW = {
    "name": "system_overview",
    "description": (
        "Get a compact overview of the current repository as a system: purpose, "
        "components and their responsibilities, boundaries, stores, external "
        "systems, primary flows, invariants, and index freshness. Call this once "
        "at the start of any substantial coding task in an unfamiliar repository."
    ),
    "parameters": {"type": "object", "properties": {}},
}

SYSTEM_ATLAS = {
    "name": "system_atlas",
    "description": (
        "Get the FULL System Atlas: complete architecture for session startup — "
        "purpose, every component with implementation paths and ownership, "
        "entrypoints, primary flows, contracts, critical invariants, failure and "
        "retry behavior, deployment, trust and async boundaries, implementation "
        "map, evidence and freshness. Prefer this over system_overview when "
        "starting work in an unfamiliar repository: the agent should know the "
        "architecture before its first coding task."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "token_budget": {
                "type": "integer",
                "description": "Optional token budget (default context.atlas_tokens, 15000)",
            }
        },
    },
}

TASK_CONTEXT = {
    "name": "task_context",
    "description": (
        "Get a task-specific system context pack for a coding goal: relevant "
        "components, primary flows, upstream/downstream dependencies, data "
        "ownership, contracts, invariants, failure/retry behavior, implementation "
        "symbols, tests, and evidence status. Call this before planning or editing "
        "for any repository-changing task — it prevents missed downstream "
        "dependencies and preserves invariants."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "goal": {
                "type": "string",
                "description": "The task/goal in natural language (e.g. 'rename the transcript field in the api response')",
            },
            "files": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Optional explicit file paths to consider",
            },
            "symbols": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Optional explicit symbol names to consider",
            },
            "token_budget": {
                "type": "integer",
                "description": "Optional hard token budget (default 8000)",
            },
        },
        "required": ["goal"],
    },
}

COMPONENT_CONTEXT = {
    "name": "component_context",
    "description": (
        "Get detailed context for one system component: responsibilities, "
        "implementation paths and symbols, owned data, dependencies, participating "
        "flows, contracts, tests, and evidence."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "component": {
                "type": "string",
                "description": "Component id or name (list from system_overview)",
            },
        },
        "required": ["component"],
    },
}

FLOW_CONTEXT = {
    "name": "flow_context",
    "description": (
        "Get one flow's detail: trigger, ordered steps with actors and operations, "
        "conditions, async boundaries, retry policies, failure outcomes, data "
        "touched, and evidence."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "flow": {
                "type": "string",
                "description": "Flow id or name (list from system_overview)",
            },
        },
        "required": ["flow"],
    },
}

IMPACT_CONTEXT = {
    "name": "impact_context",
    "description": (
        "Analyze the impact of a change before making it: affected components, "
        "flows, upstream/downstream consumers, API contracts, data, invariants, "
        "tests, and a risk level. Call this for cross-layer changes to avoid "
        "discovering broken consumers only after tests fail."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "files": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Files being changed",
            },
            "symbols": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Symbols being changed",
            },
            "diff": {
                "type": "string",
                "description": "Optional git base revision to diff against",
            },
        },
    },
}

VERIFY_CONTEXT = {
    "name": "verify_context",
    "description": (
        "Verify the system model's freshness and integrity: stale facts, dangling "
        "references, evidence gaps, drift findings, unenforced invariants, and a "
        "verdict. Call this before trusting the model after external changes, and "
        "before declaring a task complete."
    ),
    "parameters": {"type": "object", "properties": {}},
}
