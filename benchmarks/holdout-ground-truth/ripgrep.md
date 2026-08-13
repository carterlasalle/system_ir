# ripgrep
> https://github.com/BurntSushi/ripgrep | Rust | rust cli | ~77k LOC

## architecture
- crates/core — the CLI binary crate: main, flags, search orchestration (crates/core/main.rs)
- crates/core/flags — low/high-level CLI argument parsing (crates/core/flags/)
- crates/searcher — streaming search over haystacks (crates/searcher/src/searcher/)
- crates/grep — grep facade crate re-exporting matcher/printer/regex (crates/grep/)
- crates/matcher — regex matcher over bytes (crates/matcher/src/)
- crates/printer — result printing: standard, json, summary (crates/printer/src/)
- crates/ignore — recursive directory walker with gitignore support (crates/ignore/src/)
- crates/cli — shared CLI utilities: decompression, process handling (crates/cli/src/)

## entrypoints
- rg — the CLI binary (crates/core/main.rs main)
- main -> run(flags::parse()) — entry pipeline (crates/core/main.rs)
- flags::parse — CLI parse entry (crates/core/flags/parse.rs)
- SearchWorker::search — per-pattern search execution (crates/core/search.rs)
- Printer::print — output emission (crates/printer/src/standard.rs)
- grep_cli::decompress — decompression entry (crates/cli/src/decompress.rs)
- --type/--type-add — type filtering entry

## behavior
- main -> flags::parse -> run -> SearchWorkerBuilder -> SearchWorker.search — search pipeline
- SearchWorker.search -> searcher.search -> Sink matches — streaming match loop (crates/core/search.rs)
- WalkBuilder -> ignore::Walk -> file iteration with gitignore filtering (crates/ignore/src/walk.rs)
- matcher.find_at -> regex match over haystack — byte-level matching (crates/matcher/src/)
- Printer.print -> write match lines/context — output formatting (crates/printer/src/standard.rs)
- haystack streaming: read buffer -> search -> print — buffered read loop (crates/core/haystack.rs)
- parallel search: work-stealing thread pool over files — parallelism (crates/core/search.rs threads)

## state_authority
- HiArgs — fully resolved CLI configuration (crates/core/flags/hiargs.rs)
- LowArgs — raw parsed flags (crates/core/flags/lowargs.rs)
- SearchWorker — per-thread search state (crates/core/search.rs)
- Printer — output state incl. color specs
- ignore::Types — type registry for --type filtering
- Stats — match statistics when --stats (crates/printer/src/stats.rs)
- MatchBuffer — line buffering for multiline matches

## contracts
- rg <pattern> [<path>...] — search contract
- -e/--regexp <pattern> — explicit pattern flag
- -i/--ignore-case — case-insensitive flag
- -v/--invert-match — invert match flag
- -n/--line-number — line number flag
- -c/--count — match count flag
- -l/--files-with-matches — matching files only
- -L/--files-without-match — non-matching files only
- --json — JSON output contract
- -A/-B/-C <n> — context lines flags
- -g/--glob <pattern> — glob filter contract
- -t/--type <type> — file type filter
- -u/--unrestricted — disable ignore filtering
- -a/--text — treat binary as text
- --hidden — search hidden files
- -r/--replace <text> — replacement flag

## landmarks
- SearchWorker — parallel search worker (crates/core/search.rs)
- SearchWorkerBuilder — worker factory
- PatternMatcher — compiled pattern abstraction (crates/core/search.rs)
- Haystack — streaming input abstraction (crates/core/haystack.rs)
- SummaryKind — summary output modes (crates/grep/printer/)
- ColorSpecs — output color configuration
- WalkState — walker control flow (crates/ignore/src/walk.rs)
- Override — glob override matcher (crates/ignore/src/overrides.rs)

## tests
- crates/core/tests/ — CLI integration tests (rg tests)
- crates/searcher/tests/ — searcher tests
- crates/printer/tests/ — printer output tests
- crates/ignore/tests/ — walker/gitignore tests
- crates/matcher/tests/ — matcher tests
