//! Contract addresses and chains. Verified on-chain on 2026-10-23.

use alloy_primitives::{Address, address};

/// sAERO NFT (VotingEscrow, ERC721Enumerable). Lives only on Base.
pub const VOTING_ESCROW: Address = address!("bf7977e1521e9a3032bf7243ee4712f311f13915");
/// Hub Voter on Base: how much of each position is allocated to each chain.
pub const VOTER: Address = address!("49d7d4f17ec56ef8fa04e464af5d1a4942cc31e6");
/// LeafVoter, same address on every Aero chain: per-gauge votes and the allocation cooldown.
pub const LEAF_VOTER: Address = address!("382409fad8503fbcdbb27a4211551fa03e884078");
/// Multicall3 at its canonical address on every chain below.
pub const MULTICALL3: Address = address!("ca11bde05977b3631167028862be2a173976ca11");

/// sAERO and AERO both use 18 decimals.
pub const DECIMALS: u8 = 18;

/// Positions read per wallet (larger holders get the first N).
pub const TOKENS_PER_WALLET: usize = 50;
/// Max calls packed into one Multicall3 request.
pub const CALLS_PER_MULTICALL: usize = 400;
/// Give up on an RPC request after this long and try a backup.
pub const RPC_TIMEOUT_SECS: u64 = 10;

#[derive(Debug)]
pub struct Chain {
    pub id: u64,
    pub name: &'static str,
    /// Short name for `--chains`, and the suffix of its RPC override env var (RPC_BASE, ...).
    pub key: &'static str,
    /// Alchemy network name, used when ALCHEMY_API_KEY is set.
    pub alchemy: &'static str,
    pub default_rpc: &'static str,
    /// Other public RPCs, tried one at a time (rotating) when the first choice fails.
    /// Verified: correct eth_chainId and LeafVoter.allocationCooldown() = 169200.
    pub backup_rpcs: &'static [&'static str],
}

pub const HOME_CHAIN_ID: u64 = 8453;

pub const CHAINS: &[Chain] = &[
    Chain {
        id: 8453,
        name: "Base",
        key: "base",
        alchemy: "base-mainnet",
        default_rpc: "https://mainnet.base.org",
        backup_rpcs: &["https://base-rpc.publicnode.com", "https://base.drpc.org"],
    },
    Chain {
        id: 10,
        name: "Optimism",
        key: "optimism",
        alchemy: "opt-mainnet",
        default_rpc: "https://mainnet.optimism.io",
        backup_rpcs: &["https://optimism-rpc.publicnode.com", "https://optimism.drpc.org"],
    },
    Chain {
        id: 42161,
        name: "Arbitrum",
        key: "arbitrum",
        alchemy: "arb-mainnet",
        default_rpc: "https://arb1.arbitrum.io/rpc",
        backup_rpcs: &["https://arbitrum-one-rpc.publicnode.com", "https://arbitrum.drpc.org"],
    },
    Chain {
        id: 1,
        name: "Ethereum",
        key: "ethereum",
        alchemy: "eth-mainnet",
        default_rpc: "https://ethereum-rpc.publicnode.com",
        backup_rpcs: &["https://eth-mainnet.public.blastapi.io"],
    },
    Chain {
        id: 57073,
        name: "Ink",
        key: "ink",
        alchemy: "ink-mainnet",
        default_rpc: "https://rpc-gel.inkonchain.com",
        backup_rpcs: &["https://rpc-qnd.inkonchain.com", "https://ink-rpc.publicnode.com"],
    },
    Chain {
        id: 4663,
        name: "Robinhood",
        key: "robinhood",
        alchemy: "robinhood-mainnet",
        default_rpc: "https://rpc.mainnet.chain.robinhood.com",
        backup_rpcs: &["https://robinhood-rpc.publicnode.com", "https://robinhood.drpc.org"],
    },
    Chain {
        id: 5042,
        name: "Arc",
        key: "arc",
        alchemy: "arc-mainnet",
        default_rpc: "https://rpc.mainnet.arc.io",
        backup_rpcs: &["https://arc-rpc.publicnode.com", "https://arc.drpc.org"],
    },
];

pub fn home_chain() -> &'static Chain {
    CHAINS.iter().find(|c| c.id == HOME_CHAIN_ID).expect("Base is in CHAINS")
}

pub fn chain_by_id(id: u64) -> Option<&'static Chain> {
    CHAINS.iter().find(|c| c.id == id)
}

/// Finds a chain by key, name or id ("base", "Base", "8453").
pub fn find_chain(s: &str) -> Option<&'static Chain> {
    let s = s.trim();
    CHAINS
        .iter()
        .find(|c| c.key.eq_ignore_ascii_case(s) || c.name.eq_ignore_ascii_case(s) || c.id.to_string() == s)
}

/// RPCs for a chain, first choice first: its RPC_<KEY> override, then Alchemy when
/// `alchemy_key` is set, then the public RPCs as backups.
pub fn rpc_urls(chain: &Chain, env: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    let setting = |name: &str| env(name).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let mut urls = Vec::new();
    if let Some(u) = setting(&format!("RPC_{}", chain.key.to_ascii_uppercase())) {
        urls.push(u);
    }
    if let Some(k) = setting("ALCHEMY_API_KEY") {
        urls.push(format!("https://{}.g.alchemy.com/v2/{k}", chain.alchemy));
    }
    urls.push(chain.default_rpc.to_string());
    urls.extend(chain.backup_rpcs.iter().map(|u| u.to_string()));
    urls
}
