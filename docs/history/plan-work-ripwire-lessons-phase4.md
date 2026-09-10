# PLAN-ripwire-lessons-phase4

<!-- trace:v1 id=PLAN-ripwire-lessons-phase4 type=plan work=WORK-ripwire-lessons-phase4 implements=REQ-scan-classifies-registry-languages -->

<!-- trace:exempt reason=document-structure -->
## Steps

1. Add C/C++/ObjC/C#/Ruby/PHP/Lua/Swift/Kotlin to the scan `Language` enum and `classify()` extension table.
2. Keep extractors off; files are hashed and stored as FILE entities only.
3. Pin scan↔registry: every IndexSearch registry id classifies from its first extension; extractor remains false.
