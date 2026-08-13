//! Wave 9 builder/factory + module-state fact helpers shared by the
//! language extractors.
//!
//! Deterministic, side-effect free. Naming heuristics only: a fact is never
//! emitted on heuristics alone — each extractor pairs these with structural
//! evidence (returns `self`/`this`/`Self`, a `static`/`@classmethod`
//! modifier, a package-level `New*` function, a `let` binding...).

/// Module-symbol name for a file: the path stem (`requests/sessions.py` →
/// `sessions`, `src/server.ts` → `server`). The module symbol owns
/// module-level STATE facts (globals, `static` items).
pub(crate) fn module_stem(path: &str) -> String {
    let base = path.rsplit('/').next().unwrap_or(path);
    base.split('.').next().unwrap_or(base).to_string()
}

/// Whether `name` looks like a factory-function/method name for `lang`.
/// The sets mirror each ecosystem's idiom: go `New*`, java static
/// factories (`of`/`create`/`valueOf`...), python module factories
/// (`create_*`, `make_*`...), ts module factories (`createApp`,
/// `createInstance`...), rust impl constructors (`new`, `from_*`...).
pub(crate) fn is_factory_name(lang: &str, name: &str) -> bool {
    match lang {
        "go" => {
            name == "New"
                || (name.starts_with("New")
                    && name[3..].chars().next().map(|c| c.is_uppercase()).unwrap_or(false))
        }
        "java" => matches!(
            name,
            "of"
                | "create"
                | "from"
                | "valueOf"
                | "getInstance"
                | "copyOf"
                | "parse"
                | "instance"
                | "newInstance"
                | "builder"
                | "newBuilder"
        ),
        "rust" => {
            matches!(name, "new" | "default" | "create" | "parse" | "make")
                || name.starts_with("new_")
                || name.starts_with("from_")
        }
        "python" => {
            matches!(name, "create" | "make" | "build" | "new" | "factory" | "of")
                || name.starts_with("create_")
                || name.starts_with("make_")
                || name.starts_with("build_")
                || name.starts_with("new_")
                || name.starts_with("from_")
        }
        _ => {
            // typescript / javascript
            matches!(name, "create" | "make" | "build" | "new" | "of" | "factory")
                || starts_upper(name, "create")
                || starts_upper(name, "make")
                || starts_upper(name, "build")
                || starts_upper(name, "new")
        }
    }
}

/// Whether a method name participates in a fluent `.setX()` / `.withX()` /
/// `.addX()` / `.buildX()` builder chain (per-language spellings: snake_case
/// for python/rust, camelCase for ts/java, idiomatic capitalized for go).
pub(crate) fn is_builder_chain_method(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("set") || lower.starts_with("with") || lower.starts_with("add")
        || lower.starts_with("build")
}

/// Whether `name` starts with `prefix` followed by an uppercase letter or `_`
/// (`createApp`, `create_instance`).
fn starts_upper(name: &str, prefix: &str) -> bool {
    let rest = name.strip_prefix(prefix).unwrap_or("");
    !rest.is_empty()
        && rest
            .chars()
            .next()
            .map(|c| c.is_uppercase() || c == '_')
            .unwrap_or(false)
}

/// Function-valued property keys of an object-literal namespace that act as
/// factories: general factory names plus schema-builder keys (zod-style
/// `z.object`/`z.string`), axios-style `create`, vue-style `createApp`
/// (handled as module functions).
pub(crate) fn is_namespace_factory_key(name: &str) -> bool {
    is_factory_name("typescript", name)
        || matches!(
            name,
            "object"
                | "string"
                | "number"
                | "boolean"
                | "bigint"
                | "symbol"
                | "array"
                | "record"
                | "map"
                | "set"
                | "tuple"
                | "union"
                | "enum"
                | "schema"
                | "date"
                | "instance"
                | "client"
                | "app"
                | "handler"
                | "factory"
        )
}
