"""SCC plugin — registration: wire schemas to handlers, bundle the skill.

Hermes loads this via `register(ctx)` at startup. The plugin provides the
ten semantic context tools backed by the local `scc` CLI.
"""

import logging
from pathlib import Path

from . import schemas, tools

logger = logging.getLogger(__name__)

# trace:v1 id=impl.scc.hermes work=WORK-SCC-014 satisfies=REQ-SCC-IR


# trace:exempt reason=internal-detail  # plugin registration glue; behavior traced at impl.scc.hermes.tools
def register(ctx):
    """Wire schemas to handlers and register the bundled skill."""
    ctx.register_tool(
        name="system_overview",
        toolset="scc",
        schema=schemas.SYSTEM_OVERVIEW,
        handler=tools.system_overview,
    )
    ctx.register_tool(
        name="system_atlas",
        toolset="scc",
        schema=schemas.SYSTEM_ATLAS,
        handler=tools.system_atlas,
    )
    ctx.register_tool(
        name="task_context",
        toolset="scc",
        schema=schemas.TASK_CONTEXT,
        handler=tools.task_context,
    )
    ctx.register_tool(
        name="component_context",
        toolset="scc",
        schema=schemas.COMPONENT_CONTEXT,
        handler=tools.component_context,
    )
    ctx.register_tool(
        name="flow_context",
        toolset="scc",
        schema=schemas.FLOW_CONTEXT,
        handler=tools.flow_context,
    )
    ctx.register_tool(
        name="impact_context",
        toolset="scc",
        schema=schemas.IMPACT_CONTEXT,
        handler=tools.impact_context,
    )
    ctx.register_tool(
        name="verify_context",
        toolset="scc",
        schema=schemas.VERIFY_CONTEXT,
        handler=tools.verify_context,
    )
    ctx.register_tool(
        name="system_context",
        toolset="scc",
        schema=schemas.SYSTEM_CONTEXT,
        handler=tools.system_context,
    )
    ctx.register_tool(
        name="surface_map",
        toolset="scc",
        schema=schemas.SURFACE_MAP,
        handler=tools.surface_map,
    )
    ctx.register_tool(
        name="structural_source",
        toolset="scc",
        schema=schemas.STRUCTURAL_SOURCE,
        handler=tools.structural_source,
    )

    # Bundled skill: when to use which SCC operation.
    skills_dir = Path(__file__).parent / "skills"
    for child in sorted(skills_dir.iterdir()):
        skill_md = child / "SKILL.md"
        if child.is_dir() and skill_md.exists():
            ctx.register_skill(child.name, skill_md)

    logger.info("scc plugin registered: 10 tools, 1 skill")
