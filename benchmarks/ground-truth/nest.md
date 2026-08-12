# nest
> https://github.com/nestjs/nest | TypeScript | ts backend | ~117k LOC

## components
- `NestFactoryStatic` — application factory class in packages/core/nest-factory.ts
- `NestFactory` — singleton exported from packages/core/index.ts (with APP_FILTER/APP_GUARD/APP_INTERCEPTOR/APP_PIPE tokens)
- `Module` — class decorator in packages/common/decorators/modules/module.decorator.ts
- `Controller` — class decorator in packages/common/decorators/core/controller.decorator.ts
- `Injectable` — provider marker decorator in packages/common/decorators/core/injectable.decorator.ts
- `Get` — route-mapping decorator via createMappingDecorator in http/request-mapping.decorator.ts
- `Param` / `Body` / `Query` — request-extraction parameter decorators in http/route-params.decorator.ts
- `ExpressAdapter` — HTTP adapter in packages/platform-express/adapters/express-adapter.ts
- `NestExpressApplication` — platform interface in packages/platform-express/interfaces
- `ClientProxy` — abstract microservice client in packages/microservices/client/client-proxy.ts
- `MessagePattern` — microservice handler decorator in packages/microservices/decorators
- `TestingModuleBuilder` — testing harness builder in packages/testing/testing-module.builder.ts

## entrypoints
- `NestFactory.create(AppModule)` — documented bootstrap in nest-express-application.interface.ts example
- `NestFactory.createMicroservice` — microservice bootstrap (nest-factory.ts)
- `NestFactory.createApplicationContext` — headless application context bootstrap
- `app.listen(port)` — HTTP server start on NestApplication (nest-application.ts)
- `Test.createTestingModule` — testing bootstrap (packages/testing/test.ts)
- `@Controller('cats')` — sample app entry in sample/01-cats-app/src/cats/cats.controller.ts
- `packages/core/index.ts` — barrel re-exporting all public core symbols

## flows
- `NestFactory.create` -> `NestApplication` — app construction in nest-factory.ts
- `DependenciesScanner.scan` — metadata discovery into the container: `Module` decorator -> `DependenciesScanner.scan` (scanner.ts)
- `Controller` -> `RoutesResolver` — route registration onto the HTTP adapter (router/routes-resolver.ts)
- `MessagePattern` -> server — microservice handler binding to transports
- `Test.createTestingModule` -> `TestingModuleBuilder.compile` -> `TestingModule` — e2e/unit harness
- `NestContainer` -> `InstanceLoader` -> `Injector` — provider instantiation and DI resolution
- `ExpressAdapter` -> `RouterMethodFactory` — HTTP verb dispatch on the express router
- `NestFactory.create` -> `GraphInspector` — dependency-graph serialization for tooling

## ownership
- `NestContainer` — DI container owning the modules map (injector/container.ts)
- `ModulesContainer` — module registry inside NestContainer
- `ApplicationConfig` — global application-level configuration
- `InstanceLoader` — instantiates providers into the container
- `Injector` — resolves constructor dependencies
- `GraphInspector` — owns serialized dependency graph state (inspector/graph-inspector.ts)
- `DependenciesScanner` — scans module metadata into the container

## contracts
- `@Controller('cats')` — route prefix contract in sample/01-cats-app
- `@Get(':id')` — route-mapping decorator with path parameter
- `@Param('id', new ParseIntPipe())` — typed parameter extraction with pipe
- `@UseGuards(RolesGuard)` — guard application in sample cats controller
- `Test.createTestingModule` — module metadata contract in cats.controller.spec.ts
- `NestExpressApplication` — platform-typed app with `{ rawBody: true }` option
- `@EventPattern` — event-based handler decorator (microservices)
- `Transport` — enum of microservice transports (TCP, REDIS, NATS, MQTT, GRPC, RMQ)
- `APP_GUARD` — global enhancer injection token from core constants

## tests
- `packages/core/test` — mocha unit specs run via npm test
- integration/ — per-feature e2e suites (hello-world, cors, injector, auto-mock)
- sample/01-cats-app/src/cats/cats.controller.spec.ts — unit spec using createTestingModule
- sample/19-auth-jwt/e2e/app/app.e2e-spec.ts — sample e2e with Test bootstrap
- sample/02-gateways/e2e/events-gateway/gateway.e2e-spec.ts — websocket gateway e2e
- sample/26-queues/e2e/audio/audio.e2e-spec.ts — queue microservice e2e
- packages/common/test — common package test tree
- packages/core/test — core package test tree
