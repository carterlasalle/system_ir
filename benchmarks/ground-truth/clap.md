# clap
> https://github.com/clap-rs/clap | Rust | rust cli | ~84k LOC

## architecture
- `Command` — central builder struct in clap_builder/src/builder/command.rs (line 74); represents a CLI program or subcommand with args, settings, and help
- `Arg` — argument builder struct in clap_builder/src/builder/arg.rs (line 60); flags, options, and positionals
- `clap_derive` — proc-macro crate; `#[proc_macro_derive(Parser, attributes(clap, ...))]` etc. in clap_derive/src/lib.rs

## entrypoints
- `Command.new` — clap_builder/src/builder/command.rs:133; creates the root command from a name, entry of every builder API
- `Command.get_matches` — command.rs:663; parses `std::env::args_os` into `ArgMatches`
- `Command.get_matches_from` — command.rs:758; parses an arbitrary iterator of OsString
- `Command.arg` — command.rs:171; registers an `Arg` on the command
- `Command.subcommand` — command.rs:527; registers a nested `Command`
- `#[derive(Parser)]` — clap_derive/src/lib.rs:54; entry for the derive-based API
- `clap_complete.generate` — clap_complete/src/aot/generator/mod.rs:284; emits shell completion script into a writer
- `clap_mangen` — workspace crate generating man pages from a `Command` (workspace member)

## behavior
- `Command.new` — canonical builder flow: Command.new -> Command.arg -> Command.get_matches (define args, then parse argv into `ArgMatches`)
- `Command.get_matches` — parsing pipeline: Command.get_matches -> parser::Parser -> ArgMatches (clap_builder/src/parser; tokens resolved against arg mkeymap)
- `Arg.new` — argument configuration chain: Arg.new -> Arg.long -> Arg.action -> Arg.value_parser (builder/arg.rs)
- `Parser` — derive flow: #[derive(Parser)] -> CommandFactory.command -> Command.get_matches (generated code builds the Command, then parses)
- `Command.subcommand` — subcommand dispatch: Command.subcommand -> ArgMatches.subcommand (matched subcommand's matches returned from the parent matches)
- `ArgMatches.get_one` — typed value retrieval path: ArgMatches.get_one -> ValueParser -> T (arg_matches.rs:118)
- `clap_complete.generate_to` — completion generation flow: clap_complete.generate_to -> Generator.generate -> Shell (clap_complete/src/aot/generator/mod.rs:229)

## state_authority
- `ArgMatches` — owns all parsed values; `get_one`/`get_many` look up by arg Id string
- `Command` — owns its subcommands and args until consumed by `get_matches` (self-taking builder)
- `Id` — clap_builder/src/util; string identifier used as the match key for args and groups
- `mkeymap` — clap_builder/src/mkeymap.rs; key-to-arg mapping backing match lookups
- `ArgPredicate` — builder/arg_predicate.rs; condition values used by `default_value_if` chains
- `ValueParser` — builder/value_parser.rs; owns the typed parser/validator for an arg's values
- `StyledStr` — builder/styled_str.rs; owns ANSI-styled help/error text

## contracts
- `long` — long-flag name contract (`Arg::long("--flag")` form); stored as `Str` in builder/arg.rs:228
- `short('f')` — short-flag char contract (arg.rs:182)
- `Arg.required` — requiredness contract (arg.rs:755); violation yields `MissingRequiredArgument`
- `Arg.num_args` — value-count contract for options/positionals (arg.rs:1209)
- `ErrorKind.InvalidValue` — contract for values failing the arg's value parser (error/kind.rs:20)
- `ErrorKind.UnknownArgument` — contract for unrecognized flags/args (error/kind.rs:35)
- `ErrorKind.DisplayHelp` — help-request outcome treated as an Error kind (error/kind.rs:276)
- `Command.bin_name` — the binary name shown in help/version output (command.rs:1901)
- `Command.arg_required_else_help` — help-when-no-args contract (command.rs:2411)

## landmarks
- `ArgMatches` — parsed-result struct in clap_builder/src/parser/matches/arg_matches.rs (line 67); holds values keyed by arg Id
- `Parser` — derive trait in clap_builder/src/derive.rs (line 29); turns a struct into a CLI via `#[derive(Parser)]`
- `Args` — derive trait in clap_builder/src/derive.rs (line 227); groups argument fields into a reusable struct
- `Subcommand` — derive trait in clap_builder/src/derive.rs (line 262); enum variant per subcommand
- `ValueEnum` — derive trait in clap_builder/src/derive.rs (line 293); enum of possible values for an arg
- `CommandFactory` — trait in clap_builder/src/derive.rs (line 116) that builds a `Command` from a Parser type
- `FromArgMatches` — trait in clap_builder/src/derive.rs (line 130) that reconstructs the type from `ArgMatches`
- `ArgAction` — enum in clap_builder/src/builder/action.rs (line 34) controlling storage behavior (Set/Append/Count/Help/Version)
- `Error` — error struct in clap_builder/src/error/mod.rs (line 60), generic over `ErrorFormatter`
- `ErrorKind` — enum in clap_builder/src/error/kind.rs (line 4); `InvalidValue`, `UnknownArgument`, `MissingRequiredArgument`, etc.
- `ValueHint` — enum in clap_builder/src/builder/value_hint.rs (line 29) for shell-completion hints (FilePath, Url, ...)
- `ColorChoice` — enum in clap_builder/src/util/color.rs (line 6); Auto/Always/Never color output

## tests
- `clap_builder/src/builder/tests.rs` — unit tests for Command/Arg builder behavior
- `examples/` — runnable demo CLIs (derive and builder styles) used as doc-style tests
- `clap_bench` — benchmark crate in the workspace exercising parse performance
- `ErrorKind` — doctests asserting error kinds (`assert_eq!(err.kind(), ErrorKind::InvalidValue)`) in error/kind.rs
- `ArgAction` — doctests in builder/action.rs demonstrating `ArgAction::Set` usage
