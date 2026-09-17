#![forbid(unsafe_code)]

mod api;
mod assess;
mod capture;
mod cli;
mod dashboard;
mod engine;
mod monitor;
mod output;
mod packet;
mod presentation;

use clap::Parser;
use cli::{Cli, Command};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Interfaces => capture::list_interfaces(),
        Command::Monitor {
            interface,
            window_secs,
            duration_secs,
            analysis,
        } => monitor::run(interface, window_secs, duration_secs, analysis).await,
    }
}
