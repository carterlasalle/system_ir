# vue
> https://github.com/vuejs/core | TypeScript | ts frontend framework (monorepo) | ~191k LOC

## architecture
- packages/vue — the full build combining runtime + compiler (packages/vue/src/index.ts)
- packages/runtime-core — renderer-agnostic core: component, renderer, lifecycle (packages/runtime-core/src/)
- packages/runtime-dom — DOM renderer: createRenderer with nodeOps + patchProp (packages/runtime-dom/src/index.ts)
- packages/reactivity — the reactive system: ref, reactive, computed, effect (packages/reactivity/src/)
- packages/compiler-core — template compiler core (packages/compiler-core/src/)
- packages/compiler-dom — DOM-specific compiler transforms (packages/compiler-dom/src/)
- packages/compiler-sfc — SFC (single-file component) compilation (packages/compiler-sfc/src/)
- packages/server-renderer — SSR renderer (packages/server-renderer/src/)

## entrypoints
- createApp — app creation entry (packages/runtime-dom/src/index.ts)
- createRenderer — renderer factory (packages/runtime-core/src/renderer.ts)
- ref — reactive reference entry (packages/reactivity/src/ref.ts)
- reactive — reactive object entry (packages/reactivity/src/reactive.ts)
- computed — computed value entry (packages/reactivity/src/computed.ts)
- mount — app mounting entry (app.mount on the app instance)
- compile — template compile entry (packages/compiler-dom/src/index.ts)
- defineComponent — component definition entry
- h — hyperscript element creation
- watch — reactive watcher entry

## behavior
- createApp -> ensureRenderer -> createRenderer -> app.mount — app bootstrap (packages/runtime-dom/src/index.ts)
- createRenderer -> render -> patch — render pipeline (packages/runtime-core/src/renderer.ts)
- patch -> processComponent/processElement — diffing dispatch
- effect -> ReactiveEffect.run -> track/trigger — reactivity dependency collection (packages/reactivity/src/effect.ts)
- ref -> toReactive -> reactive — ref unwrapping
- compile(template) -> baseCompile -> generate render function — template compilation (packages/compiler-core/src/compile.ts)
- app.mount -> render(vnode, rootContainer) — DOM mount (packages/runtime-core/src/apiCreateApp.ts)

## state_authority
- app._instance — root component instance (packages/runtime-core/src/apiCreateApp.ts)
- renderer — the lazily created renderer singleton (packages/runtime-dom/src/index.ts)
- activeEffect — current effect being run (packages/reactivity/src/effect.ts)
- targetMap — reactive dependency graph (packages/reactivity/src/effect.ts)
- component.ctx — component render context
- compileCache — template compile cache (packages/vue/src/index.ts)
- instance.setupState — setup() returned state

## contracts
- createApp(App).mount('#app') — app mount contract
- ref(0).value — ref access contract
- reactive({count: 0}) — reactive object contract
- computed(() => ...) — computed contract
- watch(source, callback) — watcher contract
- defineComponent({setup() {...}}) — component contract
- <template>...</template> compiled SFC — template contract
- onMounted(cb) — lifecycle hook contract
- app.use(plugin) — plugin installation contract
- v-model / v-if directives — template directive contracts

## landmarks
- ReactiveEffect — the effect runner (packages/reactivity/src/effect.ts)
- createAppAPI — app factory (packages/runtime-core/src/apiCreateApp.ts)
- nodeOps — DOM node operations (packages/runtime-dom/src/nodeOps.ts)
- patchProp — DOM property patching (packages/runtime-dom/src/patchProp.ts)
- baseCompile — compiler entry (packages/compiler-core/src/compile.ts)
- compileToFunction — template to render function (packages/vue/src/index.ts)
- ensureRenderer — lazy renderer creation
- defineComponent — component definition helper

## tests
- packages/vue/__tests__/ — full-build tests
- packages/runtime-core/__tests__/ — core runtime tests
- packages/runtime-dom/__tests__/ — DOM renderer tests
- packages/reactivity/__tests__/ — reactivity tests
- packages/compiler-core/__tests__/ — compiler tests
- packages/compiler-sfc/__tests__/ — SFC compilation tests
