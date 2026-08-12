# bat
> https://github.com/sharkdp/bat | Rust | rust cli | ~39k LOC

## architecture
- `Controller` — orchestrator struct in src/controller.rs:22; drives config resolution, input opening, and printing
- `Assets` — src/assets.rs; owns compiled SyntaxSet/ThemeSet loaded from binary or cache (`from_cache` line 73, `from_binary` line 80)
- `Config` — src/config.rs:37; resolved runtime settings (language, theme, style, paging mode)
- `Input` — src/input.rs:94; wraps a file/stdin source plus metadata and description
- `App` — src/bin/bat/app.rs:55; CLI-level state bridging clap args, config, and assets

## entrypoints
- `main` — src/bin/bat/main.rs:459; process entry (`fn main`), delegates to App then Controller
- `Controller::run` — src/controller.rs:39; main printing pipeline over a list of Inputs
- `Controller::run_with_error_handler` — controller.rs:47; run variant with custom error callback
- `build_app` — src/bin/bat/clap_app.rs:21; builds the clap `Command` (all flags) and the `bat cache` subcommand
- `bat cache` — subcommand registered in clap_app.rs:746 (`Command::new("cache")`) to build/clear the syntax cache

## behavior
- `App.new` — startup flow: main -> App.new -> App.config -> Controller.run (parse args, resolve config, then print inputs)
- `App.inputs` — input opening and line emission through the selected Printer: App.inputs -> Input.open -> Printer.print -> OutputHandle
- `Controller.run` — per-input print lifecycle in printer.rs: Controller.run -> print_header -> print -> print_footer
- `Assets.from_cache` — asset loading and syntax lookup (assets.rs:92, 151): Assets.from_cache | from_binary -> get_syntax_set -> Assets.get_syntax_for_path
- `SyntaxMapping.get_syntax_for` — file-name/suffix to syntax resolution with `--map-syntax` overrides (syntax_mapping.rs:162): SyntaxMapping.get_syntax_for -> MappingTarget

## state_authority
- `Assets` — owns the compiled `SyntaxSet`/`ThemeSet` caches used by all printers
- `Config` — owns the merged flag/config-file settings passed into the Controller
- `InputDescription` — src/input.rs:15; owns per-input name/title metadata shown in headers
- `PrettyPrinter` — owns its own Assets and SyntaxMapping instances for library use
- `Controller` — owns the input list and output handle for the duration of a run

## contracts
- `--paging` — paging mode flag (`Arg::new("paging").long("paging")`, clap_app.rs:354-355), alias `-P` for `--paging=never`
- `--language` — force language flag (clap_app.rs:112-114), overrides auto-detection
- `--theme` — color theme flag (clap_app.rs:422-423)
- `--style` — header/line-number/grid/plain component selector (clap_app.rs:520-521)
- `--list-languages` — dump supported syntaxes (clap_app.rs:589-590), conflicts with `--list-themes`
- `--list-themes` — dump available themes (clap_app.rs:466-467)
- `--diff` — only show added/removed lines (clap_app.rs:171-172), conflicts with `--line-range`
- `--line-range` — restrict printed line range, conflicts with `--diff`
- `--map-syntax` — custom syntax mapping rules (clap_app.rs:397-399)
- `--no-config` — skip config file loading (clap_app.rs:613-614)
- `--config-file` — explicit config file path (clap_app.rs:659-660)
- `--highlight-line` — emphasize specific lines (clap_app.rs:136-137)

## landmarks
- `PrettyPrinter` — library-facing API in src/pretty_printer.rs:38 for syntax-highlighted output without the CLI
- `Printer` — `pub(crate) trait Printer` in src/printer.rs:83; `print_header`/`print`/`print_footer` contract
- `SimplePrinter` — non-interactive Printer impl in src/printer.rs:118
- `InteractivePrinter` — pager-aware Printer impl in src/printer.rs:480
- `SyntaxMapping` — src/syntax_mapping.rs:57; maps file names/extensions to syntax names, with custom `--map-syntax` rules
- `PagingMode` — enum in src/paging.rs:2 (Auto/Always/Never), selected by `--paging`
- `OutputHandle` — src/output.rs; write target (terminal, pager pipe, or file)

## tests
- `tests/integration_tests.rs` — end-to-end CLI tests (flags, stdin/stdout behavior)
- `tests/snapshot_tests.rs` + `tests/snapshots/` — golden output snapshots per flag combination
- `tests/test_pretty_printer.rs` — library API tests for `PrettyPrinter`
- `tests/syntax-tests` — per-language highlight correctness fixtures
- `tests/benchmarks` — performance regression tests over syntaxes
- `src/syntax_mapping.rs` `mod tests` (line 190) — unit tests for mapping rules
- `tests/github-actions.rs` — CI-specific environment tests
