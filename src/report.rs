//! The JSON report. Field names and shapes are the contract agents rely on: add fields, don't rename.

use alloy_primitives::U256;
use alloy_primitives::utils::format_units;
use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::config::DECIMALS;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Report {
    pub generated_at: Time,
    /// Set when the query was a wallet; `null` for `--token` queries.
    pub wallet: Option<String>,
    /// `false` when some chain or position could not be read; see `errors`.
    pub complete: bool,
    pub positions: Vec<Position>,
    pub errors: Vec<ReadError>,
    /// Problems that were recovered from (a backup RPC answered).
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Position {
    pub token_id: String,
    /// Base data (owner, stake, voting power). `null` with `--no-details` or if it could not be read.
    pub details: Option<Details>,
    /// Only chains this position has allocated on (cooldown or votes there), in a fixed chain order.
    pub chains: Vec<ChainPosition>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Details {
    pub owner: String,
    /// AERO locked in the position.
    pub staked: Amount,
    pub voting_power: Amount,
    /// Permanent locks do not decay and have no `lock_end`.
    pub permanent: bool,
    pub lock_end: Option<Time>,
    /// Token id this position delegates its votes to, if any.
    pub delegated_to: Option<String>,
    /// Amount the hub Voter on Base counts as allocated, and its split per chain.
    pub committed: Amount,
    pub allocations: Vec<ChainAmount>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ChainAmount {
    pub chain_id: u64,
    pub chain: &'static str,
    pub amount: Amount,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ChainPosition {
    pub chain_id: u64,
    pub chain: &'static str,
    /// Allocation the LeafVoter on this chain holds for the position.
    pub allocated: Amount,
    /// `null` if never allocated here, or with `--no-cooldowns`.
    pub cooldown: Option<Cooldown>,
    /// Gauge votes, largest first. `null` with `--no-votes` or if they could not be read.
    pub votes: Option<Vec<Vote>>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Cooldown {
    pub last_allocated: Time,
    /// last_allocated + cooldown_seconds - reduction_seconds.
    pub unlocks_at: Time,
    pub seconds_left: u64,
    /// `true` once the position may change its allocation on this chain again.
    pub ready: bool,
    pub cooldown_seconds: u64,
    pub reduction_seconds: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Vote {
    pub gauge: String,
    pub pool: Option<String>,
    /// e.g. "USDC/POD"; `null` if the token symbols could not be read.
    pub pool_name: Option<String>,
    /// "concentrated", "stable" or "volatile".
    pub pool_type: Option<&'static str>,
    pub tick_spacing: Option<i32>,
    /// Concentrated pools only, in percent (0.3 = 0.3%).
    pub fee_pct: Option<f64>,
    pub weight: Amount,
    /// Share of this position's votes on this chain, in percent.
    pub share_pct: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReadError {
    /// `null` for an error about one position rather than a chain.
    pub chain_id: Option<u64>,
    pub chain: Option<&'static str>,
    pub token_id: Option<String>,
    pub error: String,
}

/// Token amount with 18 decimals: `raw` in wei, `value` in AERO with trailing zeros trimmed.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Amount {
    pub raw: String,
    pub value: String,
}

impl Amount {
    pub fn from_u128(raw: u128) -> Self {
        Self::new(U256::from(raw))
    }

    pub fn new(raw: U256) -> Self {
        let mut value = format_units(raw, DECIMALS).unwrap_or_else(|_| raw.to_string());
        if value.contains('.') {
            value = value.trim_end_matches('0').trim_end_matches('.').to_string();
        }
        Self { raw: raw.to_string(), value }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Time {
    pub unix: u64,
    /// RFC 3339, UTC.
    pub utc: String,
}

impl Time {
    pub fn new(unix: u64) -> Self {
        let utc = i64::try_from(unix)
            .ok()
            .and_then(|t| OffsetDateTime::from_unix_timestamp(t).ok())
            .and_then(|t| t.format(&Rfc3339).ok())
            .unwrap_or_default();
        Self { unix, utc }
    }
}

/// "1d 8h 26m", "5m", "now".
pub fn format_duration(secs: u64) -> String {
    if secs == 0 {
        return "now".into();
    }
    let (d, h, m) = (secs / 86_400, secs % 86_400 / 3_600, secs % 3_600 / 60);
    let mut parts = Vec::new();
    if d > 0 {
        parts.push(format!("{d}d"));
    }
    if h > 0 {
        parts.push(format!("{h}h"));
    }
    if m > 0 || parts.is_empty() {
        parts.push(format!("{}m", m.max(1)));
    }
    parts.join(" ")
}
