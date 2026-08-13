# typer
> https://github.com/fastapi/typer | Python | python CLI framework | ~20k LOC

## architecture
- typer — the package root: Typer, run, Argument/Option (typer/)
- main.py — the core framework: Typer, run, Argument, Option (typer/main.py)
- core.py — the parameter/command internals: TyperInfo, TyperOption, TyperArgument, TyperCommand, TyperGroup (typer/core.py)
- models.py — the data models: Context, FileBinaryRead, FileText, CommandInfo, ParameterInfo (typer/models.py)
- params.py — parameter type definitions: Option, Argument (typer/params.py)
- colors.py — ANSI color constants (typer/colors.py)
- completion.py — shell completion: install_completion, completion scripts (typer/completion.py)
- cli.py — the `typer` CLI itself: init/install-completion/update-completion (typer/cli.py)
- _completion_classes.py — shell-specific completion classes: BashComplete, ZshComplete, FishComplete, PowerShellComplete (typer/_completion_classes.py)
- testing.py — the CliRunner test helper (typer/testing.py)
- docs — documentation site (docs/)

## entrypoints
- typer.Typer — the app entry (main.py)
- typer.run — run a command from a function (main.py)
- typer.Argument — argument declaration
- typer.Option — option declaration
- typer.Typer.command — decorator to register a command
- typer.Typer.callback — decorator to register a group callback
- typer.Typer.add_typer — sub-app inclusion
- app() — run the app when executed as a script
- typer.echo — CLI output helper
- typer.secho — styled output helper
- typer.prompt — interactive prompt helper
- typer.confirm — yes/no confirmation helper
- typer.Exit — exit with a code
- typer.Abort — abort the CLI run
- typer.main.get_command — produce a click Command
- typer.main.get_group — produce a click Group
- typer.completion.install_completion — install shell completion
- typer.cli — the typer meta CLI entry (cli.py)

## behavior
- Typer.command(fn) -> get_command_from_info -> click command — command registration (main.py)
- typer.run(fn) -> get_command -> click Command -> invoke — run flow (main.py)
- Typer() -> app() -> main() -> invoke — app execution (main.py)
- click context -> TyperArgument/TyperOption -> function params — parameter resolution (core.py)
- completion -> install_completion -> shell script — completion installation (completion.py)
- add_typer -> TyperGroup merge — sub-command merging (main.py)

## state_authority
- Typer — the app state: registered commands, callbacks, info (main.py)
- TyperInfo — app-level metadata (core.py)
- TyperCommand — per-command state (core.py)
- TyperGroup — the click group wrapper state (core.py)
- TyperOption — option declaration state (core.py)
- TyperArgument — argument declaration state (core.py)
- Context — the click context state (models.py)
- CliRunner — the test runner state (testing.py)

## contracts
- typer.Option(...) — option contract
- typer.Argument(...) — argument contract
- @app.command() — command contract
- @app.callback() — callback contract
- --help — help flag contract
- --version — version flag contract
- app.add_typer(sub_app) — sub-app contract
- typer.run(fn) — run contract
- typer.echo("text") — output contract
- typer.prompt("Name?") — prompt contract
- typer.confirm("Continue?") — confirm contract
- typer.Exit(code=1) — exit contract

## landmarks
- Typer — the app class (main.py)
- TyperInfo — app info model (core.py)
- TyperCommand — command model (core.py)
- TyperOption — option model (core.py)
- TyperArgument — argument model (core.py)
- Option — the option type (params.py)
- Argument — the argument type (params.py)
- run — the run helper (main.py)
- echo — the output helper (main.py)
- CliRunner — the test helper (testing.py)
- BashComplete — bash completion class (_completion_classes.py)
- ZshComplete — zsh completion class (_completion_classes.py)

## tests
- tests/test_basic.py — basic app tests
- tests/test_commands.py — command tests
- tests/test_options.py — option tests
- tests/test_arguments.py — argument tests
- tests/test_completion.py — completion tests
- tests/test_typer_cli.py — meta-CLI tests
- tests/test_callback.py — callback tests
