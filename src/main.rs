use std::process::ExitCode;
use std::str::FromStr;
use std::sync::Arc;

use aero_pos::config::{CHAINS, Chain, find_chain};
use aero_pos::{HttpTransport, Options, Reader, Rpc, Target, text, unix_now};
use alloy_primitives::{Address, U256};
use anyhow::{Result, anyhow, bail};
use clap::{Parser, Subcommand};

/// Read-only tracker for Aero sAERO positions: stake, 47h allocation cooldowns on every Aero
/// chain, and gauge votes. Prints JSON for agents (see README for the fields), or --text.
///
/// RPCs: set ALCHEMY_API_KEY (one key, all 7 chains) or RPC_BASE, RPC_OPTIMISM, ... ;
/// otherwise public RPCs are used, with one retry on a backup.
#[derive(Parser)]
#[command(version, after_help = "Exit codes: 0 = report printed (check \"complete\"), 1 = bad input or positions not readable.")]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Only these chains, comma separated (base, optimism, arbitrum, ethereum, ink, robinhood, arc).
    #[arg(long, global = true, value_delimiter = ',')]
    chains: Vec<String>,

    /// Human-readable output instead of JSON.
    #[arg(long, global = true)]
    text: bool,

    /// JSON on one line.
    #[arg(long, global = true)]
    compact: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Everything: details, cooldowns and votes.
    Status { target: Vec<String> },
    /// Stake, voting power and per-chain allocation (Base only, fastest).
    Positions { target: Vec<String> },
    /// Allocation cooldown on every chain.
    Cooldowns { target: Vec<String> },
    /// Gauge votes on every chain, with pool names.
    Votes { target: Vec<String> },
}

/// A wallet address (`0x…`), or one or more token ids (`4235`, `#4235`).
fn parse_target(args: &[String]) -> Result<Target> {
    let [first, ..] = args else { bail!("give a wallet address (0x…) or token ids (e.g. 4235)") };
    if first.starts_with("0x") || first.starts_with("0X") {
        if args.len() > 1 {
            bail!("give one wallet address, or token ids, not both");
        }
        let has_upper = first[2..].chars().any(|c| c.is_ascii_uppercase());
        let has_lower = first[2..].chars().any(|c| c.is_ascii_lowercase());
        let addr = if has_upper && has_lower {
            Address::parse_checksummed(first, None).map_err(|_| anyhow!("{first} has a bad checksum (check for a typo)"))?
        } else {
            Address::from_str(first).map_err(|_| anyhow!("{first} is not a valid address"))?
        };
        return Ok(Target::Wallet(addr));
    }
    let ids = args
        .iter()
        .flat_map(|a| a.split(','))
        .filter(|a| !a.is_empty())
        .map(|a| U256::from_str_radix(a.trim_start_matches('#'), 10).map_err(|_| anyhow!("{a} is not a token id")))
        .collect::<Result<Vec<_>>>()?;
    Ok(Target::Tokens(ids))
}

fn parse_chains(names: &[String]) -> Result<Vec<&'static Chain>> {
    if names.is_empty() {
        return Ok(CHAINS.iter().collect());
    }
    names
        .iter()
        .map(|n| {
            find_chain(n).ok_or_else(|| {
                let keys: Vec<_> = CHAINS.iter().map(|c| c.key).collect();
                anyhow!("unknown chain {n:?} (use {})", keys.join(", "))
            })
        })
        .collect()
}

async fn run(cli: &Cli) -> Result<String> {
    let (target, details, cooldowns, votes) = match &cli.command {
        Command::Status { target } => (target, true, true, true),
        Command::Positions { target } => (target, true, false, false),
        Command::Cooldowns { target } => (target, false, true, false),
        Command::Votes { target } => (target, false, false, true),
    };
    let target = parse_target(target)?;
    let opts = Options { chains: parse_chains(&cli.chains)?, details, cooldowns, votes };

    let reader = Reader::new(Rpc::from_env(Arc::new(HttpTransport::new()?)));
    let report = reader.report(target, &opts, unix_now()).await?;
    Ok(if cli.text {
        text::render(&report)
    } else if cli.compact {
        serde_json::to_string(&report)?
    } else {
        serde_json::to_string_pretty(&report)?
    })
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli).await {
        Ok(out) => {
            println!("{}", out.trim_end());
            ExitCode::SUCCESS
        }
        Err(e) => {
            // JSON callers get a machine-readable error on stdout too.
            if !cli.text {
                println!("{}", serde_json::json!({ "error": format!("{e:#}") }));
            }
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn targets() {
        let w = "0x63b4f15788aaa877a9466d2a5c1400325379931a";
        assert!(matches!(parse_target(&args(&[w])).unwrap(), Target::Wallet(_)));
        assert!(matches!(parse_target(&args(&["0x63b4F15788aaA877a9466D2A5c1400325379931a"])).unwrap(), Target::Wallet(_)));
        // Mixed case with a wrong checksum is a typo.
        assert!(parse_target(&args(&["0x63B4F15788aaA877a9466D2A5c1400325379931a"])).is_err());
        match parse_target(&args(&["#4235", "4245,4300"])).unwrap() {
            Target::Tokens(ids) => assert_eq!(ids, [U256::from(4235), U256::from(4245), U256::from(4300)]),
            Target::Wallet(_) => panic!("expected token ids"),
        }
        assert!(parse_target(&args(&[])).is_err());
        assert!(parse_target(&args(&["abc"])).is_err());
        assert!(parse_target(&args(&[w, "4235"])).is_err());
    }

    #[test]
    fn chains() {
        assert_eq!(parse_chains(&[]).unwrap().len(), 7);
        assert_eq!(parse_chains(&args(&["base", "Optimism", "42161"])).unwrap().iter().map(|c| c.id).collect::<Vec<_>>(), [8453, 10, 42161]);
        assert!(parse_chains(&args(&["nowhere"])).is_err());
    }
}
