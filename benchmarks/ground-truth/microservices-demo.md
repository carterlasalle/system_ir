# microservices-demo
> https://github.com/GoogleCloudPlatform/microservices-demo | Multi | microservices | ~22k LOC

## components
- hipstershop — proto package in protos/demo.proto (go_package github.com/GoogleCloudPlatform/microservices-demo/hipstershop)
- CartService — gRPC service in demo.proto with AddItem/GetCart/EmptyCart
- ProductCatalogService — gRPC service in demo.proto with ListProducts/GetProduct/SearchProducts
- ShippingService — gRPC service in demo.proto with GetQuote/ShipOrder
- CurrencyService — gRPC service in demo.proto with GetSupportedCurrencies/Convert
- PaymentService — gRPC service in demo.proto with Charge
- EmailService — gRPC service in demo.proto with SendOrderConfirmation
- CheckoutService — gRPC service in demo.proto with PlaceOrder
- AdService — gRPC service in demo.proto with GetAds
- RecommendationService — gRPC service in demo.proto with ListRecommendations
- frontend — Go HTTP storefront in src/frontend/main.go using gorilla/mux on port 8080
- loadgenerator — Locust load generator (src/loadgenerator/locustfile.py, WebsiteUser tasks)

## entrypoints
- src/checkoutservice/main.go — Go gRPC server entry (const listenPort = "5050")
- src/currencyservice/server.js — Node gRPC server entry (const PORT = 7000)
- src/productcatalogservice/server.go — Go gRPC server entry (const port = "3550")
- src/adservice/src/main/java/hipstershop/AdService.java — Java gRPC server entry (default PORT "9555")
- src/emailservice/email_server.py — Python gRPC server entry (default port 8080, add_insecure_port)
- src/recommendationservice/recommendation_server.py — Python gRPC server entry (default port 8080)
- src/shippingservice/main.go — Go gRPC server entry (Dockerfile ENV APP_PORT=50051)
- src/frontend/main.go — HTTP entry registering mux routes (port 8080) with session/logging middleware
- src/cartservice/src/Program.cs — .NET gRPC server entry (Dockerfile ENV ASPNETCORE_HTTP_PORTS=7070)
- src/paymentservice/server.js — Node gRPC server (HipsterShopServer.PORT from env, 50051 in manifests)

## flows
- `ListProducts` -> catalog_loader.go -> products.json — catalog loading flow (jsonpb unmarshal of products.json)
- `getCart` -> CartService.GetCart — frontend/rpc.go cart fetch flow
- `PlaceOrder` -> GetCart -> GetQuote -> Charge -> SendOrderConfirmation -> ShipOrder — checkout pipeline in checkoutservice/main.go
- `AddItem` -> CartService -> cartstore — add-to-cart flow (frontend insertCart)
- `Convert` -> CurrencyService — currency conversion flow (frontend convertCurrency)
- `GetAds` -> AdService — ad serving flow (frontend getAd with 100ms timeout)
- `ListRecommendations` -> RecommendationService — recommendation flow (frontend getRecommendations)
- `homeHandler` -> getProducts -> ProductCatalogService.ListProducts — storefront home flow

## ownership
- redis-cart — cartservice.yaml redis Deployment owning cart state (REDIS_ADDR redis-cart:6379)
- RedisCartStore — cartservice/src/cartstore/RedisCartStore.cs cart storage implementation
- ICartStore — cartservice/src/cartstore/ICartStore.cs interface owning cart persistence contract
- SpannerCartStore — Spanner-backed cart store in cartservice/src/cartstore/
- src/productcatalogservice/products.json — product catalog data owned by productcatalogservice (first product id "OLJCESPC7Z")
- cookieSessionID — frontend session cookie "shop_session-id" (cookiePrefix "shop_" + "session-id" in main.go) owning user session state
- kubernetes-manifests — per-service Deployment/Service yaml files owning cluster wiring (e.g. frontend.yaml, adservice.yaml)
- helm-chart — Helm packaging of the services
- skaffold.yaml — dev workflow orchestration for the whole demo

## contracts
- `rpc GetCart(GetCartRequest) returns (Cart)` — cart RPC contract in demo.proto
- `rpc ListProducts(Empty) returns (ListProductsResponse)` — catalog RPC contract
- `rpc PlaceOrder(PlaceOrderRequest) returns (PlaceOrderResponse)` — checkout RPC contract
- `rpc GetSupportedCurrencies(Empty) returns (GetSupportedCurrenciesResponse)` — currency RPC contract
- `containerPort: 9555` — adservice.yaml gRPC port contract
- `port: 7070` — cartservice.yaml service port contract
- `port: 5050` — checkoutservice.yaml service port contract
- `port: 50051` — paymentservice/shippingservice gRPC port contract
- `port: 3550` — productcatalogservice.yaml port contract
- `/_healthz` — frontend health-check route in main.go and readiness/liveness probes in frontend.yaml
- `GET /product/{id}` — frontend product route via r.HandleFunc(baseUrl+"/product/{id}", ...)

## tests
- src/productcatalogservice/product_catalog_test.go — catalog logic tests
- src/shippingservice/shippingservice_test.go — server tests
- src/frontend/validator/validator_test.go — form validation tests
- src/checkoutservice/money/money_test.go — monetary math tests
- src/frontend/money/money_test.go — frontend money conversion tests
- src/loadgenerator/locustfile.py — load-test scenario definitions (browseProduct/addToCart tasks)
