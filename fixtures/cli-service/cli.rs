//! cli-service: demo CLI with two subcommands, flags, and store writes.

use clap::{Arg, Command, Parser, Subcommand};

/// cli-service: demo CLI service.
#[derive(Parser)]
#[command(name = "cli-service", about = "demo CLI service")]
struct Cli {
    /// Enable paged output.
    #[arg(long)]
    paging: bool,

    /// Output format.
    #[arg(short, long)]
    format: String,

    #[command(subcommand)]
    command: Command,
}

/// Subcommands of cli-service.
#[derive(Subcommand)]
enum Command {
    /// Serve requests.
    Serve {
        /// Port to listen on.
        #[arg(long = "port", default_value_t = 8080)]
        port: u16,
    },
    /// Deploy the build.
    Deploy {
        /// Target environment.
        #[arg(short, long)]
        env: String,
    },
}

/// Builder-style CLI (clap builder API, alongside the derive example).
fn build_cli() -> Command {
    Command::new("cli-service")
        .about("demo CLI service")
        .arg(Arg::new("paging").long("paging").help("Enable paged output."))
        .arg(Arg::new("theme").short('t').long("theme").help("Color theme."))
        .arg(Arg::new("FILE"))
        .subcommand(
            Command::new("start")
                .about("Start the service")
                .arg(Arg::new("port").short('p').long("port").default_value("8080")),
        )
        .subcommand(Command::new("stop").about("Stop the service"))
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Serve { port } => println!("{port}"),
        Command::Deploy { env } => println!("{env}"),
    }
    let _app = build_cli();
}
