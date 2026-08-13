# black
> https://github.com/psf/black | Python | python tool | ~147k LOC

## architecture
- black.main — the click-based CLI entry (src/black/__init__.py main)
- Mode — the formatting configuration dataclass: line length, target versions, string normalization (src/black/mode.py)
- LineGenerator — line-to-line formatting transformer (src/black/linegen.py)
- EmptyLineTracker — blank-line insertion engine (src/black/lines.py)
- NormalizerVisitor — AST-based transform passes (src/black/trans.py)
- Cache — on-disk formatting cache (src/black/cache.py)
- Report — per-file change reporting (src/black/report.py)
- blib2to3 parser — the vendored grammar parser (src/blib2to3/)

## entrypoints
- black — the CLI command (src/black/__init__.py main)
- format_str — format a source string, return formatted string (src/black/__init__.py)
- format_file_contents — format file contents in-memory (src/black/__init__.py)
- format_file_in_place — format a file on disk (src/black/__init__.py)
- format_stdin_to_stdout — format stdin to stdout (src/black/__init__.py)
- format_cell — format one Jupyter notebook cell (src/black/__init__.py)
- python -m black — module execution entry

## behavior
- main -> format_file_in_place -> format_file_contents -> format_str — CLI formatting pipeline
- format_str -> lib2to3_parse -> LineGenerator.visit -> transform_line -> EmptyLineTracker — line transformation pipeline
- transform_line -> NormalizerVisitor transforms — per-line transform application (src/black/linegen.py)
- main -> get_cache -> read_cache/write_cache — cache load/save around formatting
- format_stdin_to_stdout -> format_str -> write to stdout — stdin pipeline
- main --check -> Report.done -> exit code — check-only mode

## state_authority
- Mode — single source of formatting truth (line length, versions, flags)
- Cache — disk-backed cache keyed by file hash (src/black/cache.py)
- Report — running stats: changed/unchanged/failed counts
- DEFAULT_LINE_LENGTH — the default 88-char line length (src/black/const.py)
- DEFAULT_EXCLUDES / DEFAULT_INCLUDES — default path filters (src/black/const.py)
- compileCache — template compile cache in the Jupyter path

## contracts
- black <src>... — positional source paths contract
- -c/--code <code> — format code string argument
- -l/--line-length <int> — line length flag (default 88)
- -t/--target-version <ver> — target Python version flag
- --check — report rather than write
- --diff — emit a diff of changes
- --color/--no-color — colored diff output
- --preview — preview style features
- --fast/--safe — AST safety check toggle
- --exclude <regex> — exclude path regex
- -q/--quiet — quiet mode
- -v/--verbose — verbose mode
- -W/--workers <n> — parallel worker count
- --config <path> — pyproject.toml config path

## landmarks
- TargetVersion — supported Python versions enum (src/black/mode.py)
- Preview — preview feature flags enum (src/black/mode.py)
- Feature — per-version feature capability enum (src/black/mode.py)
- lib2to3_parse — the parse entry (src/black/parsing.py)
- InvalidInput — parse failure exception (src/black/parsing.py)
- NothingChanged — internal no-change signal (src/black/report.py)
- StringMerger — string concatenation transform (src/black/trans.py)
- LineGenerator — the line generator class (src/black/linegen.py)

## tests
- tests/ — the black test suite
- tests/test_black.py — main CLI behavior
- tests/test_format.py — formatter unit tests
- tests/test_parse.py — parser tests
- tests/test_ipynb.py — notebook formatting tests
- tests/test_rusty.py — Rust-core comparison tests
