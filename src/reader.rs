//! Reads positions, per-chain cooldowns and gauge votes, and assembles a [`Report`].

use std::collections::HashMap;

use alloy_primitives::{Address, U256};
use alloy_sol_types::SolCall;
use anyhow::{Result, anyhow};
use futures::future::join_all;

use crate::abi::{IERC20, IGauge, ILeafVoter, IPool, IVoter, IVotingEscrow};
use crate::config::{CHAINS, Chain, LEAF_VOTER, TOKENS_PER_WALLET, VOTER, VOTING_ESCROW, chain_by_id, home_chain};
use crate::report::{Amount, ChainAmount, ChainPosition, Cooldown, Details, Position, ReadError, Report, Time, Vote};
use crate::rpc::Rpc;

pub enum Target {
    Wallet(Address),
    Tokens(Vec<U256>),
}

pub struct Options {
    /// Chains to read cooldowns and votes on.
    pub chains: Vec<&'static Chain>,
    pub details: bool,
    pub cooldowns: bool,
    pub votes: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { chains: CHAINS.iter().collect(), details: true, cooldowns: true, votes: true }
    }
}

type CallData = (Address, Vec<u8>);

fn enc<C: SolCall>(to: Address, call: C) -> CallData {
    (to, call.abi_encode())
}

fn dec<C: SolCall>(r: &Option<Vec<u8>>) -> Option<C::Return> {
    r.as_deref().and_then(|d| C::abi_decode_returns(d).ok())
}

/// Base data for one position.
struct RawDetails {
    owner: Address,
    staked: IVotingEscrow::stakedReturn,
    voting_power: U256,
    delegatee: U256,
    committed: u128,
    allocations: Vec<(u64, u128)>,
}

/// One position on one chain.
#[derive(Default)]
struct RawLeaf {
    last_allocated: u64,
    chain_allocation: u128,
    reduction: u64,
    gauges: Vec<Address>,
}

#[derive(Default, Clone)]
struct PoolInfo {
    token0: Option<Address>,
    token1: Option<Address>,
    tick_spacing: Option<i32>,
    fee: Option<u32>,
    stable: Option<bool>,
}

struct ChainRead {
    cooldown: u64,
    max_reduction: u64,
    tokens: Vec<RawLeaf>,
    /// tokenId index -> (gauge, weight); `None` if the vote reads failed.
    weights: Option<Vec<Vec<(Address, u128)>>>,
    pools: HashMap<Address, Address>,
    pool_info: HashMap<Address, PoolInfo>,
    symbols: HashMap<Address, String>,
    votes_error: Option<String>,
}

pub struct Reader {
    pub rpc: Rpc,
}

impl Reader {
    pub fn new(rpc: Rpc) -> Self {
        Self { rpc }
    }

    /// sAERO token ids owned by `owner` (2 Multicall requests on Base).
    pub async fn tokens_of(&self, owner: Address) -> Result<Vec<U256>> {
        let base = home_chain();
        let r = self.rpc.multicall(base, &[enc(VOTING_ESCROW, IVotingEscrow::balanceOfCall { owner })]).await?;
        let balance = dec::<IVotingEscrow::balanceOfCall>(&r[0]).ok_or_else(|| anyhow!("could not read the sAERO balance"))?;
        let n = balance.min(U256::from(TOKENS_PER_WALLET)).to::<usize>();
        if n == 0 {
            return Ok(Vec::new());
        }
        let calls: Vec<_> = (0..n)
            .map(|i| enc(VOTING_ESCROW, IVotingEscrow::tokenOfOwnerByIndexCall { owner, index: U256::from(i) }))
            .collect();
        let r = self.rpc.multicall(base, &calls).await?;
        r.iter()
            .map(|x| dec::<IVotingEscrow::tokenOfOwnerByIndexCall>(x).ok_or_else(|| anyhow!("could not list the sAERO positions")))
            .collect()
    }

    /// Owner, stake, voting power and per-chain allocation from Base (2 Multicall requests).
    async fn details(&self, ids: &[U256]) -> Result<Vec<Option<RawDetails>>> {
        let base = home_chain();
        let mut calls = Vec::with_capacity(ids.len() * 6);
        for &id in ids {
            calls.push(enc(VOTING_ESCROW, IVotingEscrow::ownerOfCall { tokenId: id }));
            calls.push(enc(VOTING_ESCROW, IVotingEscrow::stakedCall { tokenId: id }));
            calls.push(enc(VOTING_ESCROW, IVotingEscrow::balanceOfNFTCall { tokenId: id }));
            calls.push(enc(VOTING_ESCROW, IVotingEscrow::delegatesCall { delegator: id }));
            calls.push(enc(VOTER, IVoter::tokenStatesCall { tokenId: id }));
            calls.push(enc(VOTER, IVoter::allocationChainIdsCall { tokenId: id }));
        }
        let r = self.rpc.multicall(base, &calls).await?;

        let mut out: Vec<Option<RawDetails>> = Vec::with_capacity(ids.len());
        let mut amount_calls = Vec::new();
        let mut amount_for = Vec::new(); // (position index, chain id)
        for (i, &id) in ids.iter().enumerate() {
            let r = &r[i * 6..i * 6 + 6];
            let (Some(owner), Some(staked)) = (dec::<IVotingEscrow::ownerOfCall>(&r[0]), dec::<IVotingEscrow::stakedCall>(&r[1]))
            else {
                out.push(None);
                continue;
            };
            let hub = dec::<IVoter::tokenStatesCall>(&r[4]);
            for chain in dec::<IVoter::allocationChainIdsCall>(&r[5]).unwrap_or_default() {
                amount_calls.push(enc(VOTER, IVoter::allocationChainAmountsCall { tokenId: id, chainId: chain }));
                amount_for.push((i, chain.to::<u64>()));
            }
            out.push(Some(RawDetails {
                owner,
                staked,
                voting_power: dec::<IVotingEscrow::balanceOfNFTCall>(&r[2]).unwrap_or_default(),
                delegatee: dec::<IVotingEscrow::delegatesCall>(&r[3]).unwrap_or_default(),
                committed: hub.map(|h| h.committed).unwrap_or_default(),
                allocations: Vec::new(),
            }));
        }
        if !amount_calls.is_empty() {
            let r = self.rpc.multicall(base, &amount_calls).await?;
            for ((i, chain), x) in amount_for.into_iter().zip(&r) {
                if let (Some(d), Some(amount)) = (out[i].as_mut(), dec::<IVoter::allocationChainAmountsCall>(x)) {
                    d.allocations.push((chain, amount));
                }
            }
        }
        Ok(out)
    }

    /// Cooldown state (and gauges, with `votes`) of every position on one chain: 1 Multicall,
    /// plus up to 3 more to name the voted pools.
    async fn read_chain(&self, chain: &Chain, ids: &[U256], votes: bool) -> Result<ChainRead> {
        let per = if votes { 3 } else { 2 };
        let mut calls = vec![
            enc(LEAF_VOTER, ILeafVoter::allocationCooldownCall {}),
            enc(LEAF_VOTER, ILeafVoter::maxAccumulatedCooldownReductionCall {}),
        ];
        for &id in ids {
            calls.push(enc(LEAF_VOTER, ILeafVoter::tokenStatesCall { tokenId: id }));
            calls.push(enc(LEAF_VOTER, ILeafVoter::accumulatedCooldownReductionCall { tokenId: id }));
            if votes {
                calls.push(enc(LEAF_VOTER, ILeafVoter::allocatedGaugesCall { tokenId: id }));
            }
        }
        let r = self.rpc.multicall(chain, &calls).await?;
        let cooldown: u64 = dec::<ILeafVoter::allocationCooldownCall>(&r[0])
            .ok_or_else(|| anyhow!("LeafVoter did not answer allocationCooldown()"))?
            .to();
        let max_reduction: u64 = dec::<ILeafVoter::maxAccumulatedCooldownReductionCall>(&r[1]).map(|v| v.to()).unwrap_or(0);

        let mut tokens = Vec::with_capacity(ids.len());
        for i in 0..ids.len() {
            let r = &r[2 + i * per..2 + (i + 1) * per];
            let state = dec::<ILeafVoter::tokenStatesCall>(&r[0]).ok_or_else(|| anyhow!("could not read tokenStates({})", ids[i]))?;
            tokens.push(RawLeaf {
                last_allocated: state.lastAllocated.to(),
                chain_allocation: state.chainAllocation,
                reduction: dec::<ILeafVoter::accumulatedCooldownReductionCall>(&r[1]).map(|v| v.to()).unwrap_or(0),
                gauges: if votes { dec::<ILeafVoter::allocatedGaugesCall>(&r[2]).unwrap_or_default() } else { Vec::new() },
            });
        }

        let mut read = ChainRead {
            cooldown,
            max_reduction,
            tokens,
            weights: None,
            pools: HashMap::new(),
            pool_info: HashMap::new(),
            symbols: HashMap::new(),
            votes_error: None,
        };
        if votes {
            // Cooldowns are still useful when the vote reads fail, so this only drops the votes.
            if let Err(e) = self.read_votes(chain, ids, &mut read).await {
                read.weights = None;
                read.votes_error = Some(format!("votes not available: {e:#}"));
            }
        }
        Ok(read)
    }

    async fn read_votes(&self, chain: &Chain, ids: &[U256], read: &mut ChainRead) -> Result<()> {
        // Weights per (position, gauge), and each gauge's pool.
        let mut calls = Vec::new();
        let mut gauges: Vec<Address> = Vec::new();
        for (i, t) in read.tokens.iter().enumerate() {
            for &gauge in &t.gauges {
                calls.push(enc(LEAF_VOTER, ILeafVoter::allocationsCall { tokenId: ids[i], gauge }));
                if !gauges.contains(&gauge) {
                    gauges.push(gauge);
                }
            }
        }
        if calls.is_empty() {
            read.weights = Some(vec![Vec::new(); ids.len()]);
            return Ok(());
        }
        let n_weights = calls.len();
        calls.extend(gauges.iter().map(|&g| enc(g, IGauge::poolCall {})));
        let r = self.rpc.multicall(chain, &calls).await?;

        let mut weights = Vec::with_capacity(ids.len());
        let mut k = 0;
        for t in &read.tokens {
            let mut w = Vec::with_capacity(t.gauges.len());
            for &gauge in &t.gauges {
                w.push((gauge, dec::<ILeafVoter::allocationsCall>(&r[k]).unwrap_or(0)));
                k += 1;
            }
            weights.push(w);
        }
        read.weights = Some(weights);
        for (g, x) in gauges.iter().zip(&r[n_weights..]) {
            if let Some(pool) = dec::<IGauge::poolCall>(x) {
                read.pools.insert(*g, pool);
            }
        }

        // Pool tokens and type.
        let mut pools: Vec<Address> = read.pools.values().copied().collect();
        pools.sort();
        pools.dedup();
        if pools.is_empty() {
            return Ok(());
        }
        let calls: Vec<_> = pools
            .iter()
            .flat_map(|&p| {
                [
                    enc(p, IPool::token0Call {}),
                    enc(p, IPool::token1Call {}),
                    enc(p, IPool::tickSpacingCall {}),
                    enc(p, IPool::feeCall {}),
                    enc(p, IPool::stableCall {}),
                ]
            })
            .collect();
        let r = self.rpc.multicall(chain, &calls).await?;
        for (i, &p) in pools.iter().enumerate() {
            let r = &r[i * 5..i * 5 + 5];
            read.pool_info.insert(
                p,
                PoolInfo {
                    token0: dec::<IPool::token0Call>(&r[0]),
                    token1: dec::<IPool::token1Call>(&r[1]),
                    tick_spacing: dec::<IPool::tickSpacingCall>(&r[2]).map(|v| v.as_i32()),
                    fee: dec::<IPool::feeCall>(&r[3]).map(|v| v.to()),
                    stable: dec::<IPool::stableCall>(&r[4]),
                },
            );
        }

        // Token symbols.
        let mut tokens: Vec<Address> = read.pool_info.values().flat_map(|p| [p.token0, p.token1]).flatten().collect();
        tokens.sort();
        tokens.dedup();
        let calls: Vec<_> = tokens.iter().map(|&t| enc(t, IERC20::symbolCall {})).collect();
        let r = self.rpc.multicall(chain, &calls).await?;
        for (t, x) in tokens.iter().zip(&r) {
            if let Some(s) = dec::<IERC20::symbolCall>(x) {
                read.symbols.insert(*t, s);
            }
        }
        Ok(())
    }

    /// Reads everything `opts` asks for. Fails only if the positions themselves cannot be
    /// found on Base; a chain that does not answer becomes an entry in `errors`.
    pub async fn report(&self, target: Target, opts: &Options, now: u64) -> Result<Report> {
        let (wallet, ids) = match target {
            Target::Wallet(owner) => (Some(owner), self.tokens_of(owner).await?),
            Target::Tokens(ids) => (None, ids),
        };
        let mut errors = Vec::new();

        let details = if opts.details && !ids.is_empty() { Some(self.details(&ids).await?) } else { None };

        let wants_chains = (opts.cooldowns || opts.votes) && !ids.is_empty();
        let reads: Vec<(&Chain, Result<ChainRead>)> = if wants_chains {
            let ids = &ids;
            let futs = opts.chains.iter().map(|&c| async move { (c, self.read_chain(c, ids, opts.votes).await) });
            join_all(futs).await
        } else {
            Vec::new()
        };
        for (c, r) in &reads {
            let error = match r {
                Err(e) => Some(format!("{e:#}")),
                Ok(read) => read.votes_error.clone(),
            };
            if let Some(error) = error {
                errors.push(ReadError { chain_id: Some(c.id), chain: Some(c.name), token_id: None, error });
            }
        }

        let mut positions = Vec::with_capacity(ids.len());
        for (i, id) in ids.iter().enumerate() {
            let d = details.as_ref().and_then(|d| d[i].as_ref());
            if details.is_some() && d.is_none() {
                errors.push(ReadError {
                    chain_id: None,
                    chain: None,
                    token_id: Some(id.to_string()),
                    error: "position not found on Base (burned, merged or never minted)".into(),
                });
            }
            let mut chains = Vec::new();
            for (chain, read) in &reads {
                let Ok(read) = read else { continue };
                let t = &read.tokens[i];
                let hub_amount = d.and_then(|d| d.allocations.iter().find(|a| a.0 == chain.id)).map(|a| a.1).unwrap_or(0);
                if t.last_allocated == 0 && t.chain_allocation == 0 && t.gauges.is_empty() && hub_amount == 0 {
                    continue; // never used on this chain
                }
                chains.push(ChainPosition {
                    chain_id: chain.id,
                    chain: chain.name,
                    allocated: Amount::from_u128(t.chain_allocation),
                    cooldown: (opts.cooldowns && t.last_allocated > 0).then(|| cooldown(read, t, now)),
                    votes: if opts.votes { read.weights.as_ref().map(|w| votes(read, &w[i])) } else { None },
                });
            }
            positions.push(Position { token_id: id.to_string(), details: d.map(to_details), chains });
        }

        let warnings = self.rpc.take_warnings();
        Ok(Report {
            generated_at: Time::new(now),
            wallet: wallet.map(|w| w.to_string()),
            complete: errors.is_empty(),
            positions,
            errors,
            warnings,
        })
    }
}

fn cooldown(read: &ChainRead, t: &RawLeaf, now: u64) -> Cooldown {
    let reduction = t.reduction.min(read.max_reduction).min(read.cooldown);
    let unlocks_at = t.last_allocated + read.cooldown - reduction;
    let seconds_left = unlocks_at.saturating_sub(now);
    Cooldown {
        last_allocated: Time::new(t.last_allocated),
        unlocks_at: Time::new(unlocks_at),
        seconds_left,
        ready: seconds_left == 0,
        cooldown_seconds: read.cooldown,
        reduction_seconds: reduction,
    }
}

fn votes(read: &ChainRead, weights: &[(Address, u128)]) -> Vec<Vote> {
    let total: u128 = weights.iter().map(|w| w.1).sum();
    let mut out: Vec<Vote> = weights
        .iter()
        .map(|&(gauge, weight)| {
            let pool = read.pools.get(&gauge).copied();
            let info = pool.and_then(|p| read.pool_info.get(&p)).cloned().unwrap_or_default();
            let symbol = |t: Option<Address>| t.and_then(|t| read.symbols.get(&t).cloned());
            let pool_name = match (symbol(info.token0), symbol(info.token1)) {
                (Some(a), Some(b)) => Some(format!("{a}/{b}")),
                _ => None,
            };
            let pool_type = match (info.tick_spacing, info.stable) {
                (Some(_), _) => Some("concentrated"),
                (None, Some(true)) => Some("stable"),
                (None, Some(false)) => Some("volatile"),
                _ => None,
            };
            Vote {
                gauge: gauge.to_string(),
                pool: pool.map(|p| p.to_string()),
                pool_name,
                pool_type,
                tick_spacing: info.tick_spacing,
                fee_pct: info.tick_spacing.and(info.fee).map(|f| f as f64 / 10_000.0),
                weight: Amount::from_u128(weight),
                share_pct: if total == 0 { 0.0 } else { (weight as f64 / total as f64 * 10_000.0).round() / 100.0 },
            }
        })
        .collect();
    out.sort_by(|a, b| b.share_pct.total_cmp(&a.share_pct));
    out
}

fn to_details(d: &RawDetails) -> Details {
    let mut allocations = d.allocations.clone();
    allocations.sort_by_key(|a| CHAINS.iter().position(|c| c.id == a.0).unwrap_or(usize::MAX));
    let allocations = allocations
        .into_iter()
        .map(|(chain_id, amount)| ChainAmount {
            chain_id,
            chain: chain_by_id(chain_id).map(|c| c.name).unwrap_or("unknown"),
            amount: Amount::from_u128(amount),
        })
        .collect();
    let end: u64 = d.staked.end.to();
    Details {
        owner: d.owner.to_string(),
        staked: Amount::from_u128(d.staked.amount),
        voting_power: Amount::new(d.voting_power),
        permanent: d.staked.isPermanent,
        lock_end: (!d.staked.isPermanent && end > 0).then(|| Time::new(end)),
        delegated_to: (!d.delegatee.is_zero()).then(|| d.delegatee.to_string()),
        committed: Amount::from_u128(d.committed),
        allocations,
    }
}
