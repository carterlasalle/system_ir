# react
> https://github.com/facebook/react | TypeScript/JS | ts frontend framework (monorepo) | ~1178k LOC

## architecture
- packages/react — the public React API surface (createElement, hooks, Component) (packages/react/src/ReactClient.js)
- packages/react-dom — DOM renderer: createRoot, hydrateRoot, render (packages/react-dom/src/client/ReactDOMClient.js)
- packages/react-reconciler — the core reconciler/fiber scheduler (ReactFiberWorkLoop, ReactFiberBeginWork) (packages/react-reconciler/src/)
- packages/react-server — React Server Components / Flight protocol (ReactFlightServer.js) (packages/react-server/src/)
- packages/scheduler — the scheduler package (packages/scheduler/src/)
- packages/react-is — element type introspection (packages/react-is/)
- packages/react-devtools — devtools frontend/backend (packages/react-devtools/)

## entrypoints
- createElement — element creation entry (packages/react/src/ReactClient.js)
- createRoot — client root creation entry (packages/react-dom/src/client/ReactDOMRoot.js)
- hydrateRoot — hydration entry
- useState — hook entry (packages/react/src/ReactHooks.js)
- useEffect — effect hook entry
- useContext — context hook entry
- render — legacy root render entry (packages/react-dom/src/client/)
- createContext — context object creation
- lazy — code-splitting lazy component entry
- Suspense — suspense boundary component

## behavior
- createRoot -> createContainer -> scheduleUpdateOnFiber — root mount (packages/react-dom/src/client/ReactDOMRoot.js)
- scheduleUpdateOnFiber -> performConcurrentWorkOnRoot — update scheduling (packages/react-reconciler/src/ReactFiberWorkLoop.js)
- performUnitOfWork -> beginWork -> completeUnitOfWork — fiber traversal (ReactFiberBeginWork.js)
- commitRoot -> commitMutationEffects -> commitLayoutEffects — commit phase (ReactFiberCommitWork.js)
- dispatchSetState -> scheduleUpdateOnFiber — hook state update (ReactFiberHooks.js)
- renderWithHooks -> mountWorkInProgressHook — hook invocation (ReactFiberHooks.js)
- ReactFlightServer render -> serializeJSX -> emit model chunks — RSC serialization (ReactFlightServer.js)

## state_authority
- FiberRoot — the root fiber state (packages/react-reconciler/src/ReactFiberRoot.js)
- Fiber — per-component work unit with memoizedState
- workInProgress — the in-progress fiber tree pointer (ReactFiberWorkLoop.js)
- hook.memoizedState — per-hook state chain (ReactFiberHooks.js)
- ReactCurrentDispatcher — active dispatcher for hooks (ReactHooks.js)
- shared.pendingLanes — scheduler lane state

## contracts
- createRoot(container).render(<App/>) — root rendering contract
- useState(initialState) -> [state, setState] — state hook contract
- useEffect(effect, deps) — effect hook contract
- useContext(Context) — context consumption contract
- createContext(defaultValue) — context creation contract
- forwardRef(render) — ref forwarding contract
- memo(Component) — memoization contract
- Suspense fallback={...} — suspense fallback contract
- lazy(() => import('./Comp')) — lazy loading contract
- <App /> JSX element creation — element contract

## landmarks
- ReactFiberBeginWork — begin-work traversal (packages/react-reconciler/src/ReactFiberBeginWork.js)
- ReactFiberCommitWork — commit phase effects (ReactFiberCommitWork.js)
- ReactFiberHooks — hook implementation (ReactFiberHooks.js)
- ReactFiberLane — lane/priority model (ReactFiberLane.js)
- ReactFiberWorkLoop — the work loop (ReactFiberWorkLoop.js)
- createFiberFromElement — fiber construction
- pushEffect — effect list construction
- getWorkInProgressRoot — root state access

## tests
- packages/react/__tests__/ — react package unit tests
- packages/react-dom/__tests__/ — DOM renderer tests
- packages/react-reconciler/__tests__/ — reconciler behavior tests
- packages/react-server/__tests__/ — Flight/RSC tests
- packages/react-devtools/__tests__/ — devtools tests
