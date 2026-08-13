# zap
> https://github.com/uber-go/zap | Go | go logging library | ~30k LOC

## architecture
- logger.go — the core: Logger, New, production/development configs (logger.go)
- zapcore — the core logging engine: Core, Encoder, WriteSyncer, Entry (zapcore/)
- zapcore/core.go — the Core interface: NewCore (zapcore/core.go)
- zapcore/json_encoder.go — the JSON encoder: NewJSONEncoder (zapcore/json_encoder.go)
- zapcore/console_encoder.go — the console encoder: NewConsoleEncoder (zapcore/console_encoder.go)
- zapcore/entry.go — the log entry model: Entry (zapcore/entry.go)
- zapcore/level.go — the level model: Level, AtomicLevel (zapcore/level.go)
- field.go — structured fields: Field (field.go)
- config.go — configuration: Config, NewProductionConfig, NewDevelopmentConfig (config.go)
- sugar.go — the sugared logger: SugaredLogger (sugar.go)
- options.go — logger options: Option (options.go)
- global.go — the global logger (global.go)
- http_handler.go — HTTP handler for debug logging (http_handler.go)
- level.go — level helpers: NewAtomicLevel (level.go)
- stacktrace.go — stacktrace capture (stacktrace.go)
- sink.go — write sinks: RegisterSink (sink.go)
- buffer.go — the buffer pool: Buffer (buffer.go)

## entrypoints
- zap.New — logger from a core
- zap.NewProduction — production logger
- zap.NewDevelopment — development logger
- zap.NewNop — no-op logger
- zap.NewExample — example logger
- zap.NewAtomicLevel — atomic level entry
- zap.NewAtomicLevelAt — atomic level at a level
- log.Info — info-level logging
- log.Warn — warn-level logging
- log.Error — error-level logging
- log.Debug — debug-level logging
- log.Fatal — fatal-level logging
- log.Panic — panic-level logging
- log.With — add fields
- log.Named — add a logger name
- log.Sync — flush the logger
- log.Check — check level before logging
- zap.L — the global logger accessor
- zap.S — the global sugared logger accessor
- zapcore.NewCore — core factory
- zapcore.NewJSONEncoder — JSON encoder factory
- zapcore.NewConsoleEncoder — console encoder factory

## behavior
- log.Info(msg) -> core.Write -> encoder encode -> sink write (logger.go)
- zap.NewProduction -> build core -> logger (logger.go)
- log.With(fields) -> clone with fields -> logging (logger.go)
- AtomicLevel.SetLevel -> level change (zapcore/level.go)
- encoder.EncodeEntry -> JSON bytes -> WriteSyncer (zapcore/json_encoder.go)
- log.Check(level, msg) -> checked entry -> write (logger.go)
- SugarLogger.Info -> logger.Info via sugar (sugar.go)
- global logger replace -> zap.ReplaceGlobals (global.go)

## state_authority
- Logger — the logger state: core, options, caller skip (logger.go)
- Core — the core state: encoder, writer, level enabler (zapcore/core.go)
- Encoder — the encoder state (zapcore/encoder.go)
- Entry — the log entry state: level, time, message, fields (zapcore/entry.go)
- Field — the field state: key, type, value (field.go)
- AtomicLevel — the atomic level state (zapcore/level.go)
- WriteSyncer — the sink state (zapcore/write_syncer.go)
- Buffer — the buffer state (buffer.go)
- Config — the configuration state (config.go)
- global — the global logger state (global.go)

## contracts
- log.Info("msg") — info contract
- log.Error("msg", zap.Error(err)) — error with field contract
- log.Debug("msg") — debug contract
- log.Warn("msg") — warn contract
- log.With(zap.String("key", "value")) — with-fields contract
- zap.String("key", "value") — string field contract
- zap.Int("count", n) — int field contract
- zap.Error(err) — error field contract
- zap.Duration("d", d) — duration field contract
- zap.Object("obj", marshaler) — object field contract
- zap.NewProduction() — production config contract
- zap.NewDevelopment() — development config contract
- log.Sync() — sync contract
- log.Named("pkg") — named contract
- log.Check(zapcore.InfoLevel, "msg") — checked logging contract
- zap.ReplaceGlobals(logger) — globals contract
- {"level":"info","msg":"..."} — JSON output contract

## landmarks
- Logger — the logger struct (logger.go)
- SugaredLogger — the sugar logger (sugar.go)
- New — the logger constructor (logger.go)
- NewProduction — the production factory (logger.go)
- NewDevelopment — the development factory (logger.go)
- Field — the field type (field.go)
- AtomicLevel — the atomic level (zapcore/level.go)
- Core — the core interface (zapcore/core.go)
- Encoder — the encoder interface (zapcore/encoder.go)
- Entry — the entry struct (zapcore/entry.go)
- Config — the config struct (config.go)
- Buffer — the buffer type (buffer.go)
- CheckedEntry — the checked entry (zapcore/entry.go)

## tests
- logger_test.go — logger tests
- sugar_test.go — sugar logger tests
- field_test.go — field tests
- config_test.go — config tests
- level_test.go — level tests
- global_test.go — global logger tests
- http_handler_test.go — http handler tests
- zapcore/core_test.go — core tests
- zapcore/json_encoder_test.go — JSON encoder tests
- zapcore/console_encoder_test.go — console encoder tests
- zapcore/entry_test.go — entry tests
- zapcore/level_test.go — level tests
