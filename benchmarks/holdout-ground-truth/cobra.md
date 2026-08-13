# cobra
> https://github.com/spf13/cobra | Go | go cli lib | ~20k LOC

## architecture
- command.go — the Command type: usage, flags, subcommands, execution (command.go)
- args.go — positional argument validators (args.go)
- bash_completions.go — bash completion generation (bash_completions.go)
- powershell_completions.go — powershell completion generation
- zsh_completions.go — zsh completion generation
- fish_completions.go — fish completion generation
- completions.go — shared completion machinery (completions.go)
- active_help.go — dynamic help display (active_help.go)

## entrypoints
- NewCommand — command construction entry
- rootCmd.Execute — command tree execution entry
- rootCmd.ExecuteC — execute returning the called command
- cmd.AddCommand — subcommand registration
- cmd.Flags() — flag set access
- cmd.PersistentFlags() — inherited flag set access
- cmd.SetArgs — argument injection for testing
- cmd.SetOut/SetErr — output stream configuration
- cmd.Help — help display entry
- cmd.Version — version flag entry

## behavior
- Execute -> ExecuteC -> Find -> stripFlags -> legacyArgs — command resolution (command.go)
- findNext -> match command name against subcommands — subcommand lookup
- ExecuteC -> c.execute -> parseFlags -> Run/RunE — command execution
- validateArgs -> arg validators run — argument validation (args.go)
- InitDefaultHelpFlag/InitDefaultHelpCmd — default help injection
- GenBashCompletion -> completion script emission — completion generation (bash_completions.go)
- c.Flags().Parse -> flag parsing through pflag — flag parsing
- help command invocation -> helpFunc — help rendering

## state_authority
- Command — full command state: use/aliases/run functions/flags/subcommands (command.go)
- c.commands — the subcommand map (command.go)
- c.flags — local flag set (command.go)
- c.parent — parent command pointer
- FlagErrorFunc — flag error handler state
- Args — the active argument validator
- c.helpFlagVal — help flag state

## contracts
- cobra.Command{Use: "app", Run: func(cmd, args)} — command struct contract
- rootCmd.Execute() — execution contract
- cmd.AddCommand(sub) — subcommand contract
- cmd.Flags().StringVarP(&val, "name", "n", "", "usage") — flag registration contract
- cmd.PersistentFlags().Bool("verbose", false, "...") — persistent flag contract
- Args: cobra.ExactArgs(2) — argument count contract
- args.go validators: cobra.NoArgs / MinimumNArgs(1) — validator contracts
- RunE: func(cmd *cobra.Command, args []string) error — error-returning run contract
- SilenceUsage: true — usage suppression option
- --version — auto version flag contract
- --help — auto help flag contract

## landmarks
- Command — the command type (command.go)
- Group — command grouping struct (command.go)
- FParseErrWhitelist — parse error whitelist
- CommandDisplayNameAnnotation — display-name annotation
- FlagSetByCobraAnnotation — flag-set annotation
- GenBashCompletion — bash completion generator
- GenZshCompletion — zsh completion generator
- ExactArgs — argument count validator

## tests
- command_test.go — command behavior tests
- args_test.go — argument validator tests
- completions_test.go — completion tests
- active_help_test.go — active help tests
- bash_completions_test.go — bash completion tests
