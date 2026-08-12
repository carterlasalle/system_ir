//! CLI-surface extraction integration tests (cli-service fixture): the
//! exported System IR must expose cli-subcommand entrypoints and `cli_flags`
//! symbol attributes (argparse/click, clap, cobra, package.json), and
//! `scc flows` must mention the subcommands.

use std::collections::BTreeMap;

mod golden;

#[test]
fn cli_surface_export_and_flows() {
    let repo = golden::copy_fixture("cli-service");
    let dir = golden::workdir(repo.path());
    golden::run_ok(&dir, &["index", "--quiet"]);

    // export: entity name -> attributes
    let ir = golden::run_ok(&dir, &["export", "system-ir.json"]);
    let v: serde_json::Value =
        serde_json::from_str(&ir).expect("system-ir.json parses");
    let entities = v["entities"].as_array().expect("entities array");
    let mut attrs: BTreeMap<(String, String), serde_json::Value> = BTreeMap::new();
    for e in entities {
        let kind = e["kind"].as_str().unwrap_or("").to_string();
        let name = e["name"].as_str().unwrap_or("").to_string();
        attrs.insert((kind, name), e["attributes"].clone());
    }
    let eps_of = |kind: &str, name: &str| -> Vec<String> {
        attrs
            .get(&(kind.to_string(), name.to_string()))
            .and_then(|a| a.get("entrypoints"))
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default()
    };
    let flags_of = |kind: &str, name: &str| -> Vec<String> {
        attrs
            .get(&(kind.to_string(), name.to_string()))
            .and_then(|a| a.get("cli_flags"))
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default()
    };

    // python argparse: subcommand entrypoints on the command functions
    assert_eq!(eps_of("symbol", "serve"), vec!["cli-subcommand"]);
    assert_eq!(eps_of("symbol", "deploy"), vec!["cli-subcommand"]);
    // flags attach to the parser-owning function (sorted, deduped)
    assert_eq!(
        flags_of("symbol", "build_parser"),
        vec!["--env", "--paging", "--port", "--verbose"]
    );

    // rust clap: Parser/Subcommand structs own their flags
    assert_eq!(flags_of("symbol", "Cli"), vec!["--format", "--paging"]);
    assert_eq!(flags_of("symbol", "Command"), vec!["--env", "--port"]);
    // subcommand entrypoints land on the file entity (no variant symbols)
    let cli_rs_eps = eps_of("file", "cli.rs");
    assert!(
        cli_rs_eps.iter().filter(|k| *k == "cli-subcommand").count() >= 2,
        "cli.rs file entrypoints: {cli_rs_eps:?}"
    );

    // rust clap builder API: `Command::new(..).arg(Arg::new(..).long(..))`
    // chains attach flags to the function that builds the Command; each
    // registered subcommand emits a cli-subcommand entrypoint there.
    assert_eq!(
        flags_of("symbol", "build_cli"),
        vec!["--paging", "--port", "--theme", "-p", "-t"]
    );
    assert_eq!(
        eps_of("symbol", "build_cli"),
        vec!["cli-subcommand", "cli-subcommand"]
    );

    // go cobra: flags on the function that owns the parser (init)
    assert_eq!(flags_of("symbol", "init"), vec!["--env", "--paging", "--port"]);
    let go_eps = eps_of("file", "main.go");
    assert!(
        go_eps.iter().filter(|k| *k == "cli-subcommand").count() >= 2,
        "main.go file entrypoints: {go_eps:?}"
    );

    // package.json bin/main -> entrypoint entities
    assert_eq!(eps_of("symbol", "cli-service"), vec!["entrypoint"]);
    assert_eq!(eps_of("symbol", "dist/index.js"), vec!["entrypoint"]);

    // flows mention the subcommands (entrypoint symbols become flows)
    let flows = golden::run_ok(&dir, &["flows"]);
    assert!(flows.contains("serve"), "flows must mention serve: {flows}");
    assert!(flows.contains("deploy"), "flows must mention deploy: {flows}");
}
