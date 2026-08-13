# rxjs
> https://github.com/ReactiveX/rxjs | TypeScript | ts reactive programming lib | ~180k LOC

## architecture
- packages/rxjs — the library package: operators, subjects, creation functions (packages/rxjs/)
- src — the operator modules: one file per operator (map, filter, take, tap, switch-map, catch-error, ...) (packages/rxjs/src/)
- subject.ts — the Subject classes: Subject (packages/rxjs/src/subject.ts)
- cold-observable.ts — the ColdObservable implementation (packages/rxjs/src/cold-observable.ts)
- connectable.ts — ConnectableObservable and connectable (packages/rxjs/src/connectable.ts)
- async-subject.ts — AsyncSubject (packages/rxjs/src/async-subject.ts)
- replay-subject.ts — replaySubject and ReplaySubjectConfig (packages/rxjs/src/replay-subject.ts)
- behavior-subject.ts — behaviorSubject (packages/rxjs/src/behavior-subject.ts)
- notification.ts — the Notification type (packages/rxjs/src/notification.ts)
- util — internal utilities (packages/rxjs/src/util/)
- testing — marble testing helpers (packages/rxjs/src/testing/)
- packages/observable-polyfill — the Observable core the library builds on (@rxjs/observable-polyfill)
- apps — documentation and example apps (apps/)

## entrypoints
- Observable — the observable type (via @rxjs/observable-polyfill)
- rxjs.Subject — the subject constructor
- rxjs.of — creation: emit given values
- rxjs.from — creation: from iterable/promise
- rxjs.interval — creation: periodic emission
- rxjs.timer — creation: delayed emission
- rxjs.combine — combine latest of observables
- rxjs.concat — sequential concatenation
- rxjs.merge — merge observables
- rxjs.zip — pairwise combination
- rxjs.race — first-to-emit observable
- rxjs.pipe — pipe helper
- observable.subscribe — subscription entry
- rxjs.connectable — connectable observable factory
- rxjs.BehaviorSubject — stateful subject
- rxjs.ReplaySubject — replaying subject
- rxjs.AsyncSubject — completion-only subject
- rxjs.ConnectableObservable — connectable observable class

## behavior
- observable.subscribe -> ColdSubscriber -> observer — subscription flow (cold-observable.ts)
- pipe -> operator composition -> source.subscribe — operator pipeline (pipe.ts)
- Subject.next -> subscribers -> delivery — multicast delivery (subject.ts)
- connectable -> ConnectableObservable.connect -> shared subscription (connectable.ts)
- merge -> mergeAll -> concurrent subscription (merge.ts)
- switch-map -> inner subscription switching (switch-map.ts)
- catch-error -> error handler subscription (catch-error.ts)
- take -> limit emissions -> complete (take.ts)
- interval -> scheduler -> emission loop (interval.ts)

## state_authority
- Subject — subscriber list + current value state (subject.ts)
- ColdSubscriber — per-subscription state: AbortController, teardowns (cold-observable.ts)
- ConnectableObservable — connection state (connectable.ts)
- ConnectableConnection — active connection state (connectable.ts)
- Notification — value/error/complete payload state (notification.ts)
- ReplaySubjectConfig — replay buffer config (replay-subject.ts)
- PerSubscriptionSubjectBase — per-subscription subject state (per-subscription-subject-base.ts)
- AbortSignal — cancellation state (via polyfill)

## contracts
- of(1, 2, 3) — of creation contract
- from(array) — from creation contract
- interval(1000) — interval contract
- timer(1000) — timer contract
- observable.pipe(map(x => x)) — pipe contract
- observable.subscribe(fn) — subscribe contract
- subject.next(value) — next contract
- subject.complete() — complete contract
- subject.error(err) — error contract
- map(project) — map operator contract
- filter(predicate) — filter operator contract
- take(n) — take operator contract
- tap(sideEffect) — tap operator contract
- switchMap(project) — switchMap operator contract
- catchError(handler) — catchError operator contract
- merge(...obs) — merge contract
- combineLatest(...obs) — combineLatest contract

## landmarks
- Subject — the multicast subject (subject.ts)
- ColdObservable — the compatibility observable (cold-observable.ts)
- ConnectableObservable — the connectable (connectable.ts)
- AsyncSubject — the completion subject (async-subject.ts)
- Notification — the notification payload (notification.ts)
- pipe — the pipe helper (pipe.ts)
- ColdSubscriber — the subscription implementation (cold-observable.ts)
- behaviorSubject — the stateful subject factory (behavior-subject.ts)
- replaySubject — the replay factory (replay-subject.ts)
- PerSubscriptionSubjectBase — the base subject (per-subscription-subject-base.ts)

## tests
- packages/rxjs/src/*.spec.ts — per-operator spec files
- packages/rxjs/src/testing — marble testing helpers
- packages/rxjs/test/kernel — kernel integration tests
- packages/rxjs/src/animation-frames.spec.ts — animation frame tests
- packages/rxjs/src/connectable.spec.ts — connectable tests
