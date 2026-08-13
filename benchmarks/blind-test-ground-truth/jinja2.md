# jinja2
> https://github.com/pallets/jinja | Python | python template engine | ~30k LOC

## architecture
- jinja2 — the package root: Environment, Template, filters, loaders (src/jinja2/)
- environment.py — the engine core: Environment, Template, TemplateModule (src/jinja2/environment.py)
- lexer.py — tokenization: Lexer, TokenStream (src/jinja2/lexer.py)
- parser.py — template parsing to the AST (src/jinja2/parser.py)
- nodes.py — the template AST node types (src/jinja2/nodes.py)
- compiler.py — AST to Python bytecode compilation (src/jinja2/compiler.py)
- runtime.py — the rendering runtime: Context, Macro, Undefined (src/jinja2/runtime.py)
- loaders.py — template loading: BaseLoader, FileSystemLoader, PackageLoader, DictLoader, ChoiceLoader (src/jinja2/loaders.py)
- filters.py — built-in template filters (src/jinja2/filters.py)
- ext.py — extensions: Extension, i18n, autoescape, with, loopcontrols, debug (src/jinja2/ext.py)
- sandbox.py — the sandboxed environment (src/jinja2/sandbox.py)
- bccache.py — bytecode cache: BytecodeCache, FileSystemBytecodeCache (src/jinja2/bccache.py)
- exceptions.py — template error hierarchy: TemplateError, TemplateSyntaxError, TemplateNotFound, UndefinedError (src/jinja2/exceptions.py)

## entrypoints
- jinja2.Environment — the engine entry (environment.py)
- jinja2.Template — a compiled template (environment.py)
- Environment.from_string — template from string
- Environment.get_template — template from loader
- Environment.select_template — first-matching template from a list
- Environment.render — render a template string directly
- jinja2.Template.render — render the template
- jinja2.Template.stream — stream rendering
- jinja2.Template.generate — generator rendering
- jinja2.Template.compile — compile to Python source
- jinja2.Environment.compile_expression — compile an expression template
- jinja2.select_autoescape — autoescape policy helper
- jinja2.FileSystemLoader — loader entry
- jinja2.PackageLoader — package loader entry
- jinja2.PrefixLoader — prefix loader entry
- jinja2.ChainableUndefined — configurable undefined type
- jinja2.StrictUndefined — strict undefined type

## behavior
- Environment.get_template -> loader.load -> template.compile — template loading and compilation (environment.py)
- Template.render -> context -> runtime call -> output — rendering flow (environment.py)
- Lexer.tokenize -> Parser.parse -> nodes — parse pipeline (lexer.py/parser.py)
- Parser.parse -> compiler.compile -> code — compilation flow (parser.py/compiler.py)
- ChoiceLoader.load -> first successful loader — loader fallback chain (loaders.py)
- Environment.extend -> filters/globals registered — extension flow (environment.py)

## state_authority
- Environment — the shared engine state: filters, globals, tests, loaders, policies (environment.py)
- Context — per-render state: variables, blocks, parent context (runtime.py)
- Template — the compiled template state: code, blocks, filename (environment.py)
- BytecodeCache — the compiled-code cache (bccache.py)
- BlockReference — block rendering state (runtime.py)
- LoopContext — loop iteration state (runtime.py)
- Undefined — the undefined-variable policy state (runtime.py)

## contracts
- {{ variable }} — variable output contract
- {% block name %} — block inheritance contract
- {% extends "base.html" %} — template inheritance contract
- {% include "x.html" %} — include contract
- {% for x in items %} — loop contract
- {% if cond %} — conditional contract
- {% macro name() %} — macro definition contract
- {{ x|filter }} — filter application contract
- Environment.from_string("...") — string template contract
- Environment.get_template("index.html") — loader template contract
- Template.render(var=value) — render-with-context contract
- {% set x = 1 %} — variable assignment contract

## landmarks
- Environment — the engine core class (environment.py)
- Template — the compiled template class (environment.py)
- Lexer — the tokenizer (lexer.py)
- Parser — the parser (parser.py)
- Compiler — the code generator (compiler.py)
- Context — the render context (runtime.py)
- Macro — the macro callable (runtime.py)
- FileSystemLoader — the default loader (loaders.py)
- Extension — the extension base (ext.py)
- SandboxedEnvironment — the sandbox (sandbox.py)
- TemplateNotFound — the missing-template error (exceptions.py)
- Node — the AST base node (nodes.py)

## tests
- tests/test_lexer.py — lexer tests
- tests/test_parser.py — parser tests
- tests/test_compile.py — compiler tests
- tests/test_runtime.py — runtime tests
- tests/test_filters.py — filter tests
- tests/test_loaders.py — loader tests
- tests/test_extensions.py — extension tests
- tests/test_sandbox.py — sandbox tests
- tests/test_regression.py — regression tests
