# Phase 23 of SCC x Ripwire lessons: absorb Rule 1 self/this/super base wa…

<!-- trace:v1 id=doc.phase-23-of-scc-x-ripwire-lessons-absorb-rule-1-self-this-super-base-wa -->

<!-- trace:exempt reason=document-structure -->
## Goal

Phase 23 of SCC x Ripwire lessons: absorb Rule 1 self/this/super base walk. When self.m() or this.m() is not a sibling of the enclosing class, pin via unique class-like plus CHA (IERS_B.run self.open → IERS.open). Own-class sibling still wins. super.m() and Python super().m() never pin the enclosing class; they walk bases only (skipSelf). Two hitting bases at one level stay unresolved. Classify super() as Super. Do not pin field chains. No C++ bare-name Rule 1. Keep four context levels, 10 MCP tools, and the production ranker.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-implement-phase-23-of-scc-x-ripwire-lessons-absorb-rule-1-self-this-s — Implement Phase 23 of SCC x Ripwire lessons: absorb Rule 1 self/this/s…

<!-- trace:v1 id=REQ-implement-phase-23-of-scc-x-ripwire-lessons-absorb-rule-1-self-this-s type=requirement work=WORK-phase-23-of-scc-x-ripwire-lessons-absorb-rule-1-self-this-super-base-wa -->

Phase 23 of SCC x Ripwire lessons: absorb Rule 1 self/this/super base walk. When self.m() or this.m() is not a sibling of the enclosing class, pin via unique class-like plus CHA (IERS_B.run self.open → IERS.open). Own-class sibling still wins. super.m() and Python super().m() never pin the enclosing class; they walk bases only (skipSelf). Two hitting bases at one level stay unresolved. Classify super() as Super. Do not pin field chains. No C++ bare-name Rule 1. Keep four context levels, 10 MCP tools, and the production ranker.
