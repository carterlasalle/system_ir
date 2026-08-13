# zod
> https://github.com/colinhacks/zod | TypeScript | ts lib | ~139k LOC

## architecture
- packages/zod — the public package (packages/zod/src/index.ts)
- v4/classic — the classic zod API surface (packages/zod/src/v4/classic/external.ts)
- schemas — schema class definitions: ZodString, ZodNumber, ZodObject (packages/zod/src/v4/classic/schemas.ts)
- core — the v4 core engine: parse pipeline, error tree, registry (packages/zod/src/v4/core/)
- checks — per-type check helpers (packages/zod/src/v4/classic/checks.ts)
- parse — parse entrypoints (packages/zod/src/v4/classic/parse.ts)
- coerce — coercion schema helpers (packages/zod/src/v4/classic/coerce.ts)
- locales — error message localization (packages/zod/src/v4/locales/)

## entrypoints
- z.string — string schema factory
- z.number — number schema factory
- z.object — object schema factory
- z.array — array schema factory
- z.union — union schema factory
- z.enum — enum schema factory
- schema.parse — sync validation entry
- schema.safeParse — non-throwing validation entry
- schema.parseAsync — async validation entry
- z.infer — type extraction helper (type-level)
- z.coerce.string — coercion entry

## behavior
- ZodString.parse -> _parse -> check — string validation pipeline (packages/zod/src/v4/classic/schemas.ts)
- ZodObject._parse -> parseObjectShape -> parse each field — object validation (schemas.ts)
- parse -> runAsyncValidations/syncValidations — validation execution
- schema.safeParse -> parse -> catch error -> result object — safe result wrapping
- union._parse -> try each member — union resolution
- coerce -> primitive coerce step before validation (coerce.ts)
- formatError -> flattenError — error tree to message flattening (core/errors.ts)

## state_authority
- globalRegistry — global schema meta registry (packages/zod/src/v4/core/registry.ts)
- registry — per-schema registry
- config — global configuration incl. locale (packages/zod/src/v4/core/index.ts)
- errorTree — validation error accumulation (packages/zod/src/v4/core/error-tree.ts)
- NEVER — the unreachable schema singleton

## contracts
- z.string().min(1) — string constraint contract
- z.number().int() — number constraint contract
- z.object({ name: z.string() }) — object shape contract
- z.array(z.string()) — array element contract
- z.union([z.string(), z.number()]) — union contract
- z.enum(['a', 'b']) — enum contract
- schema.parse(data) -> typed output — parse contract
- schema.safeParse(data) -> {success, data|error} — safe parse contract
- z.infer<typeof schema> — type inference contract
- schema.pick({...}) / .omit({...}) — object shape manipulation

## landmarks
- ZodString — string schema class (packages/zod/src/v4/classic/schemas.ts)
- ZodNumber — number schema class
- ZodObject — object schema class
- ZodEffects — refinement/transform wrapper
- treeifyError — error tree builder
- formatError — error formatter
- toJSONSchema — JSON Schema export (core/json-schema-processors.ts)
- fromJSONSchema — JSON Schema import (classic/from-json-schema.ts)

## tests
- packages/zod/src/v4/classic/tests/ — classic API tests
- packages/zod/package.json test scripts — vitest-based suite
- packages/integration/ — cross-package integration tests
- packages/resolution/ — package resolution tests
- packages/treeshake/ — tree-shaking verification
