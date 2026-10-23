//! Read-only tracker for Aero (MetaDEX 03) sAERO positions: stake and voting power,
//! the 47h allocation cooldown on each Aero chain, and which gauges each position votes for.
//!
//! ```no_run
//! # async fn demo() -> anyhow::Result<()> {
//! use std::sync::Arc;
//! use aero_pos::{Options, Reader, Rpc, Target, HttpTransport};
//!
//! let reader = Reader::new(Rpc::from_env(Arc::new(HttpTransport::new()?)));
//! let wallet = "0x63b4f15788aaa877a9466d2a5c1400325379931a".parse()?;
//! let report = reader.report(Target::Wallet(wallet), &Options::default(), aero_pos::unix_now()).await?;
//! println!("{}", serde_json::to_string_pretty(&report)?);
//! # Ok(()) }
//! ```

pub mod abi;
pub mod config;
pub mod reader;
pub mod report;
pub mod rpc;
pub mod text;

pub use reader::{Options, Reader, Target};
pub use report::Report;
pub use rpc::{HttpTransport, Rpc, Transport};

pub fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
