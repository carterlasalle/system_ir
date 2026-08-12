# prettier
> https://github.com/prettier/prettier | TypeScript/JS | ts backend (tool) | ~164k LOC

## components
- `format` — main async formatting API in src/index.js
- `check` — verifies formatted output equals input (src/index.js)
- `formatWithCursor` — format returning a cursor offset (src/main/core.js)
- `coreFormat` — core pipeline entry in src/main/core.js
- `parse` — parse-only API in src/main/parse.js
- `printAstToDoc` — AST -> document conversion in src/main/ast-to-doc.js
- `prepareToPrint` — preprocessing step before doc building (ast-to-doc.js)
- `printDocToString` — document -> string printer (src/document/printer/printer.js)
- `group` — doc builder for groupable layout (src/document/builders/group.js)
- `hardline` — forced line-break doc builder (builders/line.js)
- `softline` — optional line-break doc builder
- `builders` — exported doc-builder namespace in src/document/public.js

## entrypoints
- `bin/prettier.cjs` — CLI executable declared in package.json bin
- `src/index.cjs` — package main (CJS) entry
- `src/index.js` — API entry exporting format/check/resolveConfig
- `src/cli/index.js` — CLI argument dispatch
- `src/standalone.js` — browser bundle entry (package unpkg)
- `formatFiles` — CLI bulk-format runner in src/cli/format.js
- `src/cli/context.js` — CLI context construction from argv

## flows
- `format` -> `coreFormat` -> `parse` -> `printAstToDoc` -> `printDocToString` — main pipeline
- `check` -> `format` text comparison — check-mode short-circuit
- `parse` -> `resolveParser` -> `parser.preprocess` — parser resolution (main/parse.js)
- `printAstToDoc` -> `prepareToPrint` -> doc cache Map — memoized doc building
- `formatFiles` -> `formatFile` -> `writeOutput` — per-file CLI formatting
- `listDifferent` -> `prettier.check` — diff-listing mode (cli/format.js)
- `formatFile` -> `mockable.writeFormattedFile` — write mode with mtime-preserving skip
- `normalizeInputAndOptions` -> `normalizeFormatOptions` — option normalization before core

## ownership
- `printAstToDoc` — AST-to-doc memoization across files
- `options` — normalized options object threaded through the whole pipeline
- `resolveConfig` — config file loading (src/config/resolve-config.js)
- `editorconfig` — editorconfig integration (src/config/editorconfig)
- `context.argv` — parsed CLI flags owned by the CLI context
- `mockable` — injectable fs/stream I/O layer (src/cli/mockable.js)
- `directory-ignorer` — .prettierignore handling (src/cli/directory-ignorer.js)
- `ast` — the parsed AST state flowing through prepareToPrint

## contracts
- `--write` — edit files in place (cli-options.evaluate.js)
- `--check` — verify formatting without writing
- `--list-different` — print filenames that differ from formatted output
- `--debug-check` — debug mode, incompatible with --write
- `--log-level` — CLI logging verbosity flag
- `babel` — JS parser (src/language-js/parse/babel.js)
- `typescript` — TS/TSX parser (src/language-js/parse/typescript.js)
- `postcss` — CSS/SCSS/LESS parser (src/language-css/parser-postcss.js)
- `html` — HTML parser (src/language-html)
- `yaml` — YAML parser (src/language-yaml/parser-yaml.js)
- `CATEGORY_OUTPUT` — option category constant for output flags

## tests
- tests/format — golden formatting snapshots per language (js, css, html, json, ...)
- tests/unit — unit tests for utilities and doc builders
- tests/integration — CLI-level integration tests
- tests/config — config resolution tests
- tests/dts — type-definition tests
- tests/format/js — JS/TS formatting cases
- tests/format/css — CSS formatting cases
- tests/unit/doc-builders.js — doc builder unit tests
- tests/unit/editorconfig-to-prettier.js — editorconfig mapping tests
