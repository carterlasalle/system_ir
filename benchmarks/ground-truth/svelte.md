# svelte
> https://github.com/sveltejs/svelte | TypeScript | monorepo | ~157k LOC

## components
- `compile` — component compiler entry in packages/svelte/src/compiler/index.js
- `compileModule` — runes-module compiler entry (compiler/index.js)
- `parse` — AST-only parser export (compiler/index.js)
- `parseCss` — stylesheet-only parse returning a StyleSheetFile
- `preprocess` — preprocessor pipeline exported from compiler/index.js
- `print` — AST -> code printer (compiler/print)
- `migrate` — Svelte 4 to 5 migration (compiler/migrate)
- `Component` — public component interface in src/index.d.ts
- `mount` — client render entry in internal/client/render.js
- `unmount` — component teardown (render.js)
- `hydrate` — SSR hydration entry (render.js)
- `tick` — post-update promise from internal/client/runtime.js
- `onMount` — lifecycle hook exported from src/index-client.js
- `writable` — store factory (store/shared)
- `state` — rune runtime primitive (internal/client/reactivity/sources.js)

## entrypoints
- `src/index-client.js` — package "main"/"module" entry (package.json exports ".")
- `src/index-server.js` — server-side rendering entry
- `svelte/compiler` — subpath export to compiler/index.js
- `svelte/internal` — subpath export to internal/index.js
- `svelte/store` / `svelte/motion` — subpath exports for store and motion modules
- `phases/1-parse` — compiler parse phase (Parser)
- `phases/2-analyze` — analysis phase (analyze_component)
- `phases/3-transform` — client/server transform phase

## flows
- `compile` — full compile chain: `compile` -> `_parse` -> `analyze_component` -> `transform_component`
- `parse` — CSS AST path: `parse` -> `Parser.forCss` -> `parse_stylesheet`
- `mount` — client render with effects: `mount` -> `_mount(component, options)`
- `_mount` — hydration flag reuse of mount: `hydrate` -> `_mount(..., hydrating)`
- `migrate` — converts legacy AST to runes-based code
- `preprocess` — source passed through markup/style/script preprocessors
- `state` — reactive value creation: `state` -> `source` signal + proxy (sources.js)
- `flushSync` / `fork` — batch flushing from reactivity/batch.js

## ownership
- `state` — compiler-global state (warnings, filename) in compiler/state.js
- `Batch` — client update batching manager (reactivity/batch.js)
- `mounted_components` — unmount registry in render.js
- `get_or_init_context_map` — component context storage (internal/client/context.js)
- `active_reaction` — global current-reaction pointer (runtime.js)
- `STATE_SYMBOL` — state proxy marker (internal/client/proxy.js)
- `ScopeRoot` — analyzer scope ownership (compiler phases types)
- `css` — per-component css analysis state in ComponentAnalysis

## contracts
- `$state` — rune contract referenced in internal/client/errors.js messages
- `$derived` — derived-value rune
- `$effect` — effect rune ($effect.pending() guard in errors.js)
- `$props` — props rune (props_rest_readonly error text)
- `bind:value` — binding directive handled in compiler visitors/BindDirective.js
- `bind:this` — element/component reference binding
- `bind:group` — grouped radio/checkbox binding
- `runes` — compile option toggling runes mode (compiler/index.js)
- `css` — compile option controlling css output ('external' default in legacy)
- `mount` — client mount options contract

## tests
- packages/svelte/tests/compiler-errors — compile error fixtures
- packages/svelte/tests/runtime-runes — runes-mode runtime suite
- packages/svelte/tests/runtime-legacy — legacy mode runtime suite
- packages/svelte/tests/runtime-browser — browser-environment runtime tests
- packages/svelte/tests/parser-modern — modern AST parser tests
- packages/svelte/tests/parser-legacy — legacy AST parser tests
- packages/svelte/tests/snapshot — compiler output snapshot tests
- packages/svelte/tests/server-side-rendering — SSR output tests
- packages/svelte/tests/store — store module tests
