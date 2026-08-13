// cobra-family: Command type with `AddCommand` composition builder.

package cobra

// Command is a CLI command in the tree.
type Command struct {
	Use   string
	subs  []*Command
}

// AddCommand adds subcommands to this command.
func (c *Command) AddCommand(cmds ...*Command) {
	c.subs = append(c.subs, cmds...)
}

// Execute runs the command.
func (c *Command) Execute() error {
	return nil
}
