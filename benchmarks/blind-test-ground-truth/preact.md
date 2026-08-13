# preact
> https://github.com/preactjs/preact | JavaScript | ts/js ui framework | ~20k LOC

## architecture
- src — the core: render, diff, component, hooks, options (src/)
- src/render.js — the public render/hydrate entry (src/render.js)
- src/create-element.js — JSX runtime: createElement, Fragment, createRef (src/create-element.js)
- src/component.js — the component base: BaseComponent (src/component.js)
- src/diff — the render pipeline: diff, commitRoot, children diffing (src/diff/)
- src/hooks — the hooks implementation (src/hooks/)
- src/create-context.js — context: createContext (src/create-context.js)
- src/create-portal.js — portals: createPortal (src/create-portal.js)
- src/options.js — the global options hook (src/options.js)
- compat — the React compatibility layer: preact/compat (compat/)
- debug — debug helpers: preact/debug (debug/)
- hooks — the hooks package: preact/hooks (hooks/)

## entrypoints
- preact.render — render a vnode into the DOM
- preact.hydrate — hydrate existing DOM
- preact.createElement — create a vnode
- preact.cloneElement — clone with new props
- preact.Component — the component base class
- preact.createContext — context provider/consumer
- preact.createRef — ref creation
- preact.Fragment — the fragment component
- preact.createPortal — render into another DOM node
- preact.toChildArray — normalize children
- preact.options — the options hook
- preact.h — the hyperscript alias
- preact/hooks: useState — state hook
- preact/hooks: useEffect — effect hook
- preact/hooks: useReducer — reducer hook
- preact/hooks: useMemo — memo hook
- preact/hooks: useCallback — callback hook
- preact/hooks: useRef — ref hook
- preact/hooks: useContext — context hook

## behavior
- render(vnode, dom) -> diff -> commitRoot -> DOM — render pipeline (src/render.js, src/diff/)
- setState -> enqueueRender -> rerender — state update flow (src/component.js)
- diff -> diffChildren -> placeChild — child reconciliation (src/diff/)
- useState -> hookState -> component rerender — hook lifecycle (hooks/src/index.js)
- createContext -> Provider -> consumer subscription (src/create-context.js)
- unmount -> componentWillUnmount -> teardown (src/diff/index.js)
- options._render -> vnode created -> options hooks (src/options.js)

## state_authority
- BaseComponent — the component instance state: props, state, context (src/component.js)
- vnode — the vnode state: type, props, key, ref (src/create-element.js)
- hookState — per-component hook state (hooks/src/index.js)
- Context — the context state (src/create-context.js)
- options — the global options state (src/options.js)
- commitQueue — the pending commit queue (src/diff/index.js)
- DOM — the mounted DOM state

## contracts
- render(<App/>, document.body) — render contract
- hydrate(<App/>, dom) — hydrate contract
- createElement("div", {id: "x"}) — element creation contract
- <Component prop={value}/> — component contract
- <Fragment>...</Fragment> — fragment contract
- createPortal(vnode, dom) — portal contract
- <Context.Provider value={v}> — provider contract
- <Context.Consumer>{(v) => ...}</Context.Consumer> — consumer contract
- class X extends Component — class component contract
- function X(props) — function component contract
- useState(initial) — state hook contract
- useEffect(fn, deps) — effect hook contract
- useReducer(reducer, init) — reducer hook contract
- useMemo(fn, deps) — memo hook contract
- useRef(initial) — ref hook contract
- key={id} — keyed child contract
- ref={el => ...} — ref contract

## landmarks
- render — the render function (src/render.js)
- hydrate — the hydrate function (src/render.js)
- createElement — the element factory (src/create-element.js)
- BaseComponent — the component base (src/component.js)
- enqueueRender — the render scheduler (src/component.js)
- diff — the diff function (src/diff/index.js)
- commitRoot — the commit function (src/diff/index.js)
- createContext — the context factory (src/create-context.js)
- createPortal — the portal factory (src/create-portal.js)
- Fragment — the fragment component (src/create-element.js)
- options — the options hook (src/options.js)
- getDomSibling — the DOM sibling helper (src/component.js)

## tests
- test/render.test.js — render tests
- test/hydrate.test.js — hydrate tests
- test/component.test.js — component tests
- test/hooks.test.js — hooks tests
- test/context.test.js — context tests
- test/fragments.test.js — fragment tests
- test/keys.test.js — keyed children tests
- compat/test/ — compat layer tests
- hooks/test/ — hooks package tests
