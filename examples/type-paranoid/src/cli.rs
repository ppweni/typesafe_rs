use crate::engine::Config;
use clap::{Args, Parser, Subcommand};
use std::net::SocketAddr;

#[derive(Parser)]
#[command(version, about = "TypeParanoid: trust types, question traffic.")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// List host interfaces. Does not capture traffic or call TypeSafe.
    Interfaces,
    /// Passively observe TCP/UDP traffic on one Ethernet-compatible interface.
    Monitor {
        #[arg(short, long)]
        interface: String,
        #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u64).range(1..=3600))]
        window_secs: u64,
        /// Stop after this many seconds. Otherwise, run until Ctrl-C.
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=86400))]
        duration_secs: Option<u64>,
        #[command(flatten)]
        analysis: Analysis,
    },
}

#[derive(Args)]
pub struct Analysis {
    /// Send candidate metadata to TypeSafe. Without this flag, use local rules only.
    #[arg(long)]
    pub assess: bool,
    /// Emit newline-delimited JSON on stdout (diagnostics remain on stderr).
    #[arg(long)]
    pub json: bool,
    /// Show a live terminal dashboard with traffic graphs (requires a terminal).
    #[arg(long, conflicts_with = "json")]
    pub pretty: bool,
    /// Large outbound transfer threshold, in observed TCP/UDP payload bytes per window.
    #[arg(long, default_value_t = 10_485_760, value_parser = clap::value_parser!(u64).range(1..))]
    pub upload_bytes: u64,
    /// Minimum outbound bytes for novelty and baseline-spike rules.
    #[arg(long, default_value_t = 262_144, value_parser = clap::value_parser!(u64).range(1..))]
    pub min_upload_bytes: u64,
    /// Suppress candidates for this destination IP:port. Repeat for multiple peers.
    #[arg(long)]
    pub expected_peer: Vec<SocketAddr>,
}

impl Analysis {
    pub fn config(&self) -> Config {
        Config {
            upload_bytes: self.upload_bytes,
            min_upload_bytes: self.min_upload_bytes,
            expected: self.expected_peer.iter().copied().collect(),
            ..Config::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_has_no_demo_and_requires_explicit_capture_interface() {
        assert!(Cli::try_parse_from(["type-paranoid", "demo"]).is_err());
        assert!(Cli::try_parse_from(["type-paranoid", "monitor"]).is_err());
        assert!(
            Cli::try_parse_from([
                "type-paranoid",
                "monitor",
                "-i",
                "eth0",
                "--window-secs",
                "0"
            ])
            .is_err()
        );
        let cli = Cli::try_parse_from(["type-paranoid", "monitor", "-i", "eth0"]).unwrap();
        let Command::Monitor { analysis, .. } = cli.command else {
            panic!("expected monitor")
        };
        assert!(!analysis.assess);
    }

    #[test]
    fn pretty_is_opt_in_and_conflicts_with_json() {
        let cli =
            Cli::try_parse_from(["type-paranoid", "monitor", "-i", "en1", "--pretty"]).unwrap();
        let Command::Monitor { analysis, .. } = cli.command else {
            panic!("expected monitor")
        };
        assert!(analysis.pretty);
        assert!(
            Cli::try_parse_from([
                "type-paranoid",
                "monitor",
                "-i",
                "en1",
                "--pretty",
                "--json"
            ])
            .is_err()
        );
    }
}
