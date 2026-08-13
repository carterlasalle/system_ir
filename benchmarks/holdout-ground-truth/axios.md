# axios
> https://github.com/axios/axios | TypeScript/JS | ts lib | ~95k LOC

## architecture
- lib/axios.js — the module entry: createInstance factory + default instance
- core/Axios — the Axios class: request dispatch, interceptors (lib/core/Axios.js)
- core/dispatchRequest — request dispatch pipeline (lib/core/dispatchRequest.js)
- core/InterceptorManager — request/response interceptor chains (lib/core/InterceptorManager.js)
- core/AxiosHeaders — header container (lib/core/AxiosHeaders.js)
- core/AxiosError — error type with response payload (lib/core/AxiosError.js)
- adapters — transport adapters: xhr, http, fetch (lib/adapters/)
- defaults — default config and transformers (lib/defaults/index.js)
- helpers — utility helpers (lib/helpers/)

## entrypoints
- axios(config) — default-instance request entry (lib/axios.js)
- axios.get/post/put/delete — verb shortcuts on the default instance
- axios.create(config) — new instance factory (lib/axios.js)
- Axios.request — instance request dispatch (lib/core/Axios.js)
- instance.interceptors.request.use — request interceptor registration
- instance.interceptors.response.use — response interceptor registration
- axios.all — promise-batch helper
- axios.spread — callback spread helper
- axios.CancelToken — cancellation token entry
- axios.isAxiosError — error type guard

## behavior
- createInstance -> new Axios -> bind request — instance construction (lib/axios.js)
- Axios.request -> _request -> mergeConfig -> dispatchRequest — request pipeline (lib/core/Axios.js)
- dispatchRequest -> transformRequest -> adapter(request) — transport dispatch (lib/core/dispatchRequest.js)
- InterceptorManager.forEach -> chain execution — interceptor chain run (lib/core/InterceptorManager.js)
- adapter rejection -> AxiosError -> response error shape — error normalization
- dispatchRequest -> transformResponse -> settle — response settlement
- CancelToken -> reason -> promise rejection — cancellation flow (lib/cancel/CancelToken.js)

## state_authority
- instance.defaults — merged default config (lib/core/Axios.js)
- instance.interceptors.request/response — interceptor managers
- context.defaults — per-instance config store
- mergeConfig — config precedence resolution (lib/core/mergeConfig.js)
- defaults — module-wide default config (lib/defaults/index.js)
- AxiosHeaders — header state per request

## contracts
- axios({ method: 'get', url: '/user' }) — config-object request contract
- axios.get('/user', { params: {...} }) — verb shortcut contract
- .then(response => response.data) — response contract (data/status/headers)
- axios.create({ baseURL, timeout }) — instance config contract
- interceptor use(fn, fn) — interceptor contract
- cancelToken: new CancelToken(cb) — cancellation contract
- transformRequest/transformResponse — transformer config contracts
- axios.isAxiosError(err) — error discrimination contract
- validateStatus — status validation config contract
- headers common get/accept — default header contract (lib/defaults/index.js)

## landmarks
- Axios — the core request class (lib/core/Axios.js)
- InterceptorManager — interceptor chain manager
- AxiosHeaders — header container class
- AxiosError — error class (lib/core/AxiosError.js)
- CanceledError — cancellation error (lib/cancel/CanceledError.js)
- CancelToken — cancellation source (lib/cancel/CancelToken.js)
- mergeConfig — config merge helper (lib/core/mergeConfig.js)
- buildURL — URL construction helper (lib/helpers/buildURL.js)
- HttpStatusCode — status code name map (lib/helpers/HttpStatusCode.js)

## tests
- test/ — the axios test suite
- test/unit/ — unit tests
- test/module/ — module system tests (ESM/CJS)
- test/types/ — TypeScript type tests
- test/specs/ — behavioral specs
