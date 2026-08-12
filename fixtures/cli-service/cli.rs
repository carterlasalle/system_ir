//! cli-service: demo CLI with two subcommands, flags, and store writes.

use clap::{Parser, Subcommand};

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

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Serve { port } => println!("{port}"),
        Command::Deploy { env } => println!("{env}"),
    }
}
