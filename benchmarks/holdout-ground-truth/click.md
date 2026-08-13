# click
> https://github.com/pallets/click | Python | python lib | ~39k LOC

## architecture
- click.core — the command framework: Command, Group, Context, Parameter (src/click/core.py)
- click.decorators — decorator API: command, group, option, argument (src/click/decorators.py)
- click.parser — the option/argument parser (src/click/parser.py)
- click.termui — terminal UI: prompt, confirm, progressbar, echo (src/click/termui.py)
- click.types — parameter type conversions (src/click/types.py)
- click.exceptions — exception hierarchy: UsageError, BadParameter, ClickException (src/click/exceptions.py)
- click.formatting — help text formatting (src/click/formatting.py)
- click.globals — current-context tracking (src/click/globals.py)

## entrypoints
- click.command — decorator turning a function into a Command (src/click/decorators.py)
- click.group — decorator creating a Group (src/click/decorators.py)
- click.option — option decorator (src/click/decorators.py)
- click.argument — argument decorator (src/click/decorators.py)
- Command.main — command invocation entry (src/click/core.py)
- Command.invoke — command body execution (src/click/core.py)
- Group.command — register a subcommand on a group (src/click/core.py)
- Context.invoke — invoke a command within a context (src/click/core.py)
- cli() — the created callable entrypoint

## behavior
- Command.main -> Command.parse_args -> Command.invoke — invocation pipeline (src/click/core.py)
- Group.invoke -> _parse_args -> CommandCollection/Group subcommand dispatch — subcommand routing
- Context.push/pop — context scoping during execution (src/click/core.py)
- Command.parse_args -> _OptionParser.parse_args — argument parsing (src/click/core.py)
- make_pass_decorator -> Context.find_object — object passing through contexts
- parser.parse_args -> split_opts -> consume value — option tokenization (src/click/parser.py)
- Command.get_help -> HelpFormatter.render — help generation (src/click/core.py)

## state_authority
- Context — per-invocation state: params, meta, command stack (src/click/core.py)
- ParameterSource — enum recording where a param value came from (src/click/core.py)
- _OptionParser — parsing state machine (src/click/parser.py)
- Context._meta — arbitrary per-invocation metadata dict
- current_context — thread-local active context (src/click/globals.py)
- Context.params — resolved parameter values

## contracts
- @click.command() — command declaration contract
- @click.option('--count', type=int) — option declaration contract
- @click.argument('filename') — positional argument contract
- @click.group() — group declaration contract
- --help — auto help flag contract
- ctx.params — parameter access contract inside a command
- @click.pass_context — context injection contract
- @click.version_option(version=...) — version flag contract
- type=click.Choice([...]) — choice validation contract

## landmarks
- CommandCollection — flat multi-command runner (src/click/core.py)
- BadParameter — parameter validation failure (src/click/exceptions.py)
- UsageError — usage error base (src/click/exceptions.py)
- ClickException — user-facing error with exit code (src/click/exceptions.py)
- confirm — yes/no prompt helper (src/click/termui.py)
- prompt — text input helper (src/click/termui.py)
- progressbar — terminal progress bar (src/click/termui.py)
- secho — styled echo (src/click/termui.py)
- shell_completion — shell completion support (src/click/shell_completion.py)

## tests
- tests/ — the click test suite
- tests/test_basic.py — core command behavior
- tests/test_arguments.py — argument handling
- tests/test_options.py — option parsing
- tests/test_commands.py — command/group behavior
- tests/test_parser.py — parser unit tests
