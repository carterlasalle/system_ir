# shadcn-ui
> https://github.com/shadcn-ui/ui | TypeScript | nextjs fullstack | ~550k LOC

## architecture
- `init` — CLI Command in packages/shadcn/src/commands/init.ts
- `add` — CLI Command in packages/shadcn/src/commands/add.ts
- `apply` — preset-apply Command in commands/apply.ts
- `build` — build-token Command in commands/build.ts
- `@shadcn/react` — component source package under packages/react
- `@shadcn/helpers` — shared utilities package under packages/helpers
- `v4` — apps/v4 Next.js docs app workspace (turbo workspace apps/*)

## entrypoints
- `npx shadcn init` — CLI init documented in packages/shadcn/README.md
- `shadcn add` — CLI add documented in packages/shadcn/README.md
- `npx shadcn create` — alias entry for init (README)
- `next dev --turbopack --port 4000` — v4 app dev script (apps/v4/package.json)
- `scripts/build-registry.mts` — registry build entry in apps/v4
- `apps/v4/app/(app)/docs/[[...slug]]/page.tsx` — docs page route
- `apps/v4/app/(app)/blocks/[...categories]/page.tsx` — blocks explorer route
- `apps/v4/app/api/search/route.ts` — search API route using fumadocs createFromSource

## behavior
- `build-registry.mts` — registry pipeline: `registry:build` -> `@shadcn/react` build -> `build-registry.mts` -> `registry.json`
- `add` command -> fetch item address -> apply component to project
- `components.json` — init command: write `components.json` -> install dependencies
- `packages/shadcn/src/commands/apply.ts` — preset-apply command: `apply` command -> preset -> registry items
- `apps/v4/lib/source.ts` — docs content: docs page -> `lib/source.ts` -> MDX content tree
- `createFromSource` — doc search: search route -> `createFromSource(source)` (api/search/route.ts)
- `capture-registry.mts` — component screenshots: `registry:capture` -> `capture-registry.mts`
- `capture-explore.mts` — explore page captures: `explore:capture` -> `capture-explore.mts`

## state_authority
- `apps/v4/registry.json` — generated registry index of all items
- `apps/v4/components.json` — component configuration file
- `apps/v4/lib/registry.ts` — registry access helpers
- `apps/v4/lib/config.ts` — registry/theme configuration
- `registry/new-york-v4` — new-york style component sources (ui/, blocks/, hooks/)
- `registry/bases` — base style variants (aria, base, radix)
- `apps/v4/lib/themes.ts` — theme definitions

## contracts
- `"button"` — registry item name in apps/v4/registry.json
- `apps/v4/registry/new-york-v4/ui/card.tsx` — registry item name in apps/v4/registry.json
- `apps/v4/registry/new-york-v4/ui/dialog.tsx` — registry item name in apps/v4/registry.json
- `npx shadcn init` — CLI usage contract in packages/shadcn/README.md
- `shadcn add` — CLI usage contract in README
- `--yes` — skip-confirmation flag on the add command
- `data-slot="dialog"` — slot contract in registry/new-york-v4/ui/dialog.tsx
- `data-slot="card"` — slot contract in ui/card.tsx
- `[[...slug]]` — catch-all docs route segment under app/(app)/docs

## landmarks
- `Button` — registry component in apps/v4/registry/new-york-v4/ui/button.tsx
- `buttonVariants` — cva variant helper exported alongside Button
- `Card` — registry component (with CardHeader/CardTitle/CardContent/CardFooter) in card.tsx
- `Dialog` — registry component (with DialogTrigger/DialogContent/DialogClose) in dialog.tsx
- `apps/v4/source.config.ts` — docs content configuration for apps/v4 (fumadocs-mdx)

## tests
- packages/shadcn/src/commands/add.test.ts — CLI add command tests
- packages/shadcn/src/commands/init.test.ts — CLI init command tests
- packages/shadcn/src/commands/build.test.ts — build command tests
- packages/shadcn/src/commands/apply.test.ts — apply command tests
- packages/shadcn/test — CLI test fixtures (message-scroller, questionnaire, use-render)
- apps/v4/registry/calendar.test.ts — registry component unit tests
- apps/v4/registry/config.test.ts — registry config tests
- packages/tests — cross-package integration test workspace
