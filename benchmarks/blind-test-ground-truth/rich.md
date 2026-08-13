# rich
> https://github.com/Textualize/rich | Python | python terminal formatting lib | ~90k LOC

## architecture
- rich — the package root: Console, Table, Panel, Live, progress (rich/)
- console.py — the core rendering engine: Console, ConsoleOptions, RenderHook, Group (rich/console.py)
- table.py — the table renderable: Table, Column, Row (rich/table.py)
- live.py — live-updating display: Live, _RefreshThread (rich/live.py)
- progress.py — progress display: Progress, ProgressColumn, BarColumn, TextColumn (rich/progress.py)
- panel.py — the panel renderable: Panel (rich/panel.py)
- tree.py — the tree renderable: Tree (rich/tree.py)
- markdown.py — markdown rendering: Markdown (rich/markdown.py)
- syntax.py — syntax highlighting: Syntax (rich/syntax.py)
- traceback.py — exception traceback rendering: Traceback (rich/traceback.py)
- log.py — the log renderable: Log (rich/log.py)
- text.py — text and style handling: Text, Span (rich/text.py)
- color.py — color model: Color, ColorSystem (rich/color.py)
- style.py — style model: Style (rich/style.py)
- measure.py — measurement: measure, Measurement (rich/measure.py)
- control.py — terminal control sequences (rich/control.py)
- _windows.py — Windows console support (rich/_windows.py)

## entrypoints
- rich.console.Console — the console entry (console.py)
- Console.print — print a renderable
- Console.print_json — pretty-print JSON
- Console.log — log with timestamp
- Console.rule — horizontal rule
- Console.status — status spinner context
- Console.input — interactive input
- Console.export_text — export rendered output as text
- rich.table.Table — the table entry
- rich.live.Live — the live display entry
- rich.progress.Progress — the progress entry
- rich.panel.Panel — the panel entry
- rich.tree.Tree — the tree entry
- rich.markdown.Markdown — the markdown entry
- rich.syntax.Syntax — the syntax entry
- rich.traceback.Traceback — the traceback entry
- rich.log.Log — the log entry
- rich.get_console — the default console accessor
- rich.print — print to the default console
- rich.pretty.pprint — pretty printer entry

## behavior
- Console.print -> render -> measure -> segments -> output — render pipeline (console.py)
- Console.log -> time -> renderable -> output — log flow (console.py)
- Live.start -> refresh loop -> stop — live update loop (live.py)
- Progress.update -> refresh -> bar render — progress flow (progress.py)
- Traceback.parse -> render -> console — traceback rendering (traceback.py)
- Syntax.highlight -> tokenize -> render — highlighting flow (syntax.py)
- Table.add_row -> render columns -> measure — table rendering (table.py)

## state_authority
- Console — the terminal state: theme, color system, file, width (console.py)
- ConsoleOptions — per-render options state (console.py)
- Theme — the style theme (theme.py)
- Style — the style state (style.py)
- Color — the color state (color.py)
- Live — the live display state (live.py)
- Progress — the progress state: tasks, columns (progress.py)
- Table — the table state: columns, rows, styles (table.py)
- Text — the text buffer state (text.py)
- ConsoleThreadLocals — thread-local console state (console.py)

## contracts
- console.print("text") — print contract
- console.print(table) — renderable print contract
- console.log("event") — log contract
- console.rule("title") — rule contract
- console.status("working...") — status contract
- Table().add_column("name") — column contract
- Table().add_row("a", "b") — row contract
- table = Table(title="T") — table title contract
- Panel("content", title="P") — panel contract
- Live(renderable) — live display contract
- Progress().add_task("work", total=100) — task contract
- console.export_text() — export contract
- [bold red]...[/] — markup style contract
- [link url]...[/link] — link contract

## landmarks
- Console — the core class (console.py)
- RenderableType — the renderable protocol (console.py)
- Table — the table class (table.py)
- Live — the live class (live.py)
- Progress — the progress class (progress.py)
- Panel — the panel class (panel.py)
- Tree — the tree class (tree.py)
- Markdown — the markdown class (markdown.py)
- Syntax — the syntax class (syntax.py)
- Traceback — the traceback class (traceback.py)
- Text — the text class (text.py)
- Style — the style class (style.py)
- Color — the color class (color.py)
- Measure — the measurement protocol (measure.py)

## tests
- tests/test_console.py — console tests
- tests/test_table.py — table tests
- tests/test_text.py — text tests
- tests/test_style.py — style tests
- tests/test_color.py — color tests
- tests/test_panel.py — panel tests
- tests/test_live.py — live tests
- tests/test_progress.py — progress tests
- tests/test_tree.py — tree tests
- tests/test_markdown.py — markdown tests
- tests/test_syntax.py — syntax tests
- tests/test_traceback.py — traceback tests
