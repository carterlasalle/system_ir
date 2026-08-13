# lit
> https://github.com/lit/lit | TypeScript | ts web components monorepo | ~100k LOC

## architecture
- packages/lit — the main package: LitElement + lit-html re-exports (packages/lit/)
- packages/lit-element — the LitElement class package (packages/lit-element/)
- packages/lit-html — the html template engine (packages/lit-html/)
- packages/reactive-element — the ReactiveElement base class (packages/reactive-element/)
- packages/lit-html/src/lit-html.ts — the template engine core: html, render, nothing (packages/lit-html/src/lit-html.ts)
- packages/lit-html/src/directives — the directive collection: repeat, class-map, style-map, if-defined, live, keyed, map, range (packages/lit-html/src/directives/)
- packages/lit-element/src/lit-element.ts — the LitElement class (packages/lit-element/src/lit-element.ts)
- packages/reactive-element/src/reactive-element.ts — the reactive base class (packages/reactive-element/src/reactive-element.ts)
- packages/lit/src/decorators — the public decorators: custom-element, property, state, query, query-all (packages/lit/src/decorators/)
- packages/context — the Context protocol package (packages/context/)
- packages/react — React integration bindings (packages/react/)
- packages/task — the Task controller package (packages/task/)
- packages/localize — localization packages (packages/localize/)
- packages/labs — experimental packages (packages/labs/)

## entrypoints
- lit.html — tagged template entry
- lit.render — render a template into a container
- lit.LitElement — the element base class
- lit.ReactiveElement — the reactive base class
- lit.css — the css tagged template
- lit.svg — the svg tagged template
- lit.unsafeHTML — render raw HTML
- lit.nothing — the nothing sentinel
- lit.noChange — the no-change sentinel
- lit.render(html`...`, container) — render contract
- lit-html.render — the lit-html render entry
- lit-element.customElement — class decorator
- lit-element.property — property decorator
- lit-element.state — state decorator
- lit-element.query — query decorator
- lit-element.queryAll — query-all decorator
- reactive-element.ReactiveController — the controller interface

## behavior
- render(template, container) -> commit -> update DOM — render pipeline (lit-html.ts)
- html`...` -> template parse -> part evaluation (lit-html.ts)
- LitElement.update -> reactive properties -> render -> commit — element update cycle (lit-element.ts)
- ReactiveElement.requestUpdate -> update cycle schedule (reactive-element.ts)
- property decorator -> attribute reflection -> property change (reactive-element.ts)
- repeat directive -> keyed reconciliation (directives/repeat.ts)
- class-map directive -> class attribute diffing (directives/class-map.ts)
- directive -> directive instance lifecycle (directive.ts)

## state_authority
- ReactiveElement — the element state: properties, attributes, update cycle (reactive-element.ts)
- LitElement — the rendered element state (lit-element.ts)
- Template — the compiled template state (lit-html.ts)
- Part — the template part state (lit-html.ts)
- ReactiveController — the controller state contract (reactive-element.ts)
- ChildPart — the child part state (lit-html.ts)
- AttributePart — the attribute part state (lit-html.ts)
- Directive — the directive instance state (directive.ts)
- Context — the context protocol state (packages/context/)

## contracts
- html`<p>${x}</p>` — html template contract
- render(html`...`, container) — render contract
- @customElement("my-el") — custom element contract
- @property({type: String}) — property decorator contract
- @state() — state decorator contract
- @query("#x") — query decorator contract
- @queryAll("li") — query-all decorator contract
- class MyEl extends LitElement — element class contract
- connectedCallback() — lifecycle contract
- disconnectedCallback() — lifecycle contract
- attributeChangedCallback() — attribute lifecycle contract
- reactiveController.hostConnected() — controller lifecycle contract
- html`<slot></slot>` — slot contract
- nothing — nothing sentinel contract
- unsafeHTML(html) — unsafe html contract

## landmarks
- LitElement — the element class (lit-element.ts)
- ReactiveElement — the reactive base (reactive-element.ts)
- html — the template tag (lit-html.ts)
- render — the render function (lit-html.ts)
- css — the css tag (lit-html.ts)
- svg — the svg tag (lit-html.ts)
- nothing — the nothing sentinel (lit-html.ts)
- noChange — the no-change sentinel (lit-html.ts)
- ReactiveController — the controller interface (reactive-element.ts)
- customElement — the decorator (lit/src/decorators/custom-element.ts)
- property — the decorator (lit/src/decorators/property.ts)
- state — the decorator (lit/src/decorators/state.ts)
- query — the decorator (lit/src/decorators/query.ts)
- Directive — the directive base (lit-html/src/directive.ts)
- repeat — the repeat directive (lit-html/src/directives/repeat.ts)

## tests
- packages/lit-html/src/test/lit-html_test.ts — template engine tests
- packages/reactive-element/src/test/reactive-element_test.ts — reactive element tests
- packages/lit-element/src/test/lit-element_test.ts — element tests
- packages/lit/src/test/lit_test.ts — package tests
- packages/lit-html/src/directives/*.test.ts — directive tests
- packages/context/src/test/ — context tests
- packages/task/src/test/ — task tests
