// zerolog-family: `New(w io.Writer) Logger` package factory, value-receiver
// `Context` chain (`.Str()` returns Context), and a package-level var
// (mutable module state).

package zerolog

import "io"

// Logger writes structured log events.
type Logger struct {
	w       io.Writer
	context Context
}

// New returns a Logger writing to w.
func New(w io.Writer) Logger {
	return Logger{w: w}
}

// NewConsoleWriter returns a console-format writer.
func NewConsoleWriter(opts ...func(*ConsoleWriter)) ConsoleWriter {
	return ConsoleWriter{}
}

// Context carries the log context for chained calls.
type Context struct {
	log Logger
}

// Str adds a string field to the context.
func (c Context) Str(key string, val string) Context {
	return c
}

// Logger returns the logger with the accumulated context.
func (c Context) Logger() Logger {
	return c.log
}

// With starts a new context from the logger.
func (l Logger) With() Context {
	return Context{log: l}
}

var DefaultLogger = New(io.Discard)
