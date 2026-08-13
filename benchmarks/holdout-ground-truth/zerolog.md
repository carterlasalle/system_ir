# zerolog
> https://github.com/rs/zerolog | Go | go lib (logging) | ~23k LOC

## architecture
- log.go — the Logger/Event core: leveled logging, field chaining (log.go)
- context.go — the Context builder: With() chains (context.go)
- console.go — console (human-readable) writer (console.go)
- array.go — array field types (array.go)
- globals.go — global logger singleton and hooks (globals.go)
- internal/json — the JSON encoding helpers (internal/json/)
- hook.go — log hooks (hook.go)
- cmd/zerolog — the CLI helpers package (cmd/)

## entrypoints
- zerolog.New(writer) — logger construction entry
- log.Logger — the global logger (globals.go)
- log.Info — info level event entry
- log.Debug — debug level event entry
- log.Error — error level event entry
- log.Warn — warn level event entry
- log.Fatal — fatal level event entry
- logger.With() — contextual logger derivation
- zerolog.SetGlobalLevel — global level filtering
- event.Msg — message emission terminating a chain
- event.Send — event emission without message
- log.Sample — sampling entry

## behavior
- Info().Msg("...") -> newEvent -> Level -> write JSON — event construction and emission
- With().Str("k","v").Logger() -> clone with context fields — sub-logger creation (context.go)
- New -> newContext -> newEvent with context — logger build (log.go)
- SetGlobalLevel -> global level gate in newEvent — level filtering (log.go)
- Event.write -> appendJSON -> writer.Write — JSON serialization
- ConsoleWriter -> Write -> human formatting (console.go)
- Hook.Run on level events — hook dispatch (hook.go)
- Sample -> sampler decision per event — sampling (log.go)

## state_authority
- Logger — the logger state: context fields, output writer (log.go)
- Context — the field-builder state (context.go)
- Event — the in-flight event state: fields buffer, level (log.go)
- globals — global logger and level state (globals.go)
- hooks — global hook registry (hook.go)
- TimestampFieldName/LevelFieldName — configurable field names (log.go)
- ConsoleWriter — console formatting state (console.go)

## contracts
- zerolog.New(os.Stderr).With().Timestamp().Logger() — logger construction contract
- log.Info().Msg("hello") — leveled message contract
- log.Error().Err(err).Msg("failed") — error field contract
- log.Info().Str("k", "v").Int("n", 1).Msg("...") — typed field chaining contract
- log.Info().Dict("d", dict) — dict field contract
- log.Info().Array("a", array) — array field contract
- zerolog.SetGlobalLevel(zerolog.InfoLevel) — level gate contract
- log.Logger = log.With().Str("service", "x").Logger() — global logger replacement
- log.Info().Timestamp() — timestamp field contract
- event.Enabled() — level pre-check contract
- hook.Run(e, level, msg) — hook signature contract

## landmarks
- Logger — the logger struct (log.go)
- Event — the event struct (log.go)
- Context — the context builder (context.go)
- ConsoleWriter — console output (console.go)
- BasicSampler — fixed-rate sampler (sampler.go)
- RandomSampler — randomized sampler (sampler.go)
- Hooks — the hook interface (hook.go)
- Level — the level type with String() (log.go)
- array.Array — array builder (array.go)

## tests
- log_test.go — logger/event tests
- context_test.go — context chain tests
- console_test.go — console writer tests
- array_test.go — array field tests
- internal/json tests — JSON encoding tests
- benchmark_test.go — benchmarks
