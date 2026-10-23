//! End-to-end tests of `Reader::report` against fake chains (no network).
//! `cargo test -- --ignored` also runs one test against the real chains.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use aero_pos::abi::{Call3Result, IERC20, IGauge, ILeafVoter, IPool, IVoter, IVotingEscrow, aggregate3Call};
use aero_pos::config::{CHAINS, LEAF_VOTER, VOTER, VOTING_ESCROW, rpc_urls};
use aero_pos::report::{Amount, format_duration};
use aero_pos::{Options, Reader, Rpc, Target, Transport};
use alloy_primitives::aliases::{I24, U24, U48};
use alloy_primitives::{Address, U256, address};
use alloy_sol_types::SolCall;
use anyhow::{Result, anyhow, bail};
use futures::future::BoxFuture;

const WALLET: Address = address!("63b4f15788aaa877a9466d2a5c1400325379931a");
const T0: u64 = 1_792_682_497; // real allocation of #4235 on Base
const H: u64 = 3600;
const E18: u128 = 1_000_000_000_000_000_000;

const G1: Address = address!("1dd07350114baf9fa8d9caf91cd5a3480d34ec6f");
const G2: Address = address!("00000000000000000000000000000000000000b2");
const G3: Address = address!("00000000000000000000000000000000000000c3");
const P1: Address = address!("c4d5e62261648a3ceb127924f741c0929f040f13");
const P2: Address = address!("00000000000000000000000000000000000000d2");
const USDC: Address = address!("833589fcd6edb6e08f4c7c32d4f71b54bda02913");
const POD: Address = address!("ed664536023d8e4b1640c394777d34abaff1df8f");
const VOL: Address = address!("00000000000000000000000000000000000000e5");

/// Fake RPCs for every chain: answers each Multicall3 sub-call from a table.
#[derive(Default)]
struct FakeChains {
    url_chain: HashMap<String, u64>,
    answers: HashMap<(u64, Address, Vec<u8>), Vec<u8>>,
    failing: HashSet<String>,
    hits: Mutex<Vec<String>>,
}

impl FakeChains {
    fn new(env: &dyn Fn(&str) -> Option<String>) -> Self {
        let mut f = Self::default();
        for c in CHAINS {
            for u in rpc_urls(c, env) {
                f.url_chain.insert(u, c.id);
            }
        }
        f
    }

    fn answer<C: SolCall>(&mut self, chain: u64, to: Address, call: C, ret: C::Return) {
        self.answers.insert((chain, to, call.abi_encode()), C::abi_encode_returns(&ret));
    }

    fn fail_chain(&mut self, chain: u64) {
        let urls: Vec<_> = self.url_chain.iter().filter(|(_, c)| **c == chain).map(|(u, _)| u.clone()).collect();
        self.failing.extend(urls);
    }
}

impl Transport for FakeChains {
    fn eth_call<'a>(&'a self, url: &'a str, _to: Address, data: Vec<u8>) -> BoxFuture<'a, Result<Vec<u8>>> {
        Box::pin(async move {
            self.hits.lock().unwrap().push(url.to_string());
            let chain = *self.url_chain.get(url).ok_or_else(|| anyhow!("unexpected url {url}"))?;
            if self.failing.contains(url) {
                bail!("HTTP 429");
            }
            let req = aggregate3Call::abi_decode(&data)?;
            let results: Vec<Call3Result> = req
                .calls
                .iter()
                .map(|c| match self.answers.get(&(chain, c.target, c.callData.to_vec())) {
                    Some(ret) => Call3Result { success: true, returnData: ret.clone().into() },
                    None => Call3Result { success: false, returnData: Default::default() },
                })
                .collect();
            Ok(aggregate3Call::abi_encode_returns(&results))
        })
    }
}

/// #4235: 1.5 AERO, permanent. Base: 1.0 allocated, 75% USDC/POD (concentrated) + 25% VOL/USDC
/// (volatile). Optimism: 0.5 allocated to one gauge whose pool can't be read. Nothing elsewhere.
fn world(env: &dyn Fn(&str) -> Option<String>) -> FakeChains {
    let mut f = FakeChains::new(env);
    let id = U256::from(4235);
    let base = 8453;
    f.answer(base, VOTING_ESCROW, IVotingEscrow::balanceOfCall { owner: WALLET }, U256::from(1));
    f.answer(base, VOTING_ESCROW, IVotingEscrow::tokenOfOwnerByIndexCall { owner: WALLET, index: U256::ZERO }, id);
    f.answer(base, VOTING_ESCROW, IVotingEscrow::ownerOfCall { tokenId: id }, WALLET);
    f.answer(
        base,
        VOTING_ESCROW,
        IVotingEscrow::stakedCall { tokenId: id },
        IVotingEscrow::stakedReturn { amount: 3 * E18 / 2, end: U48::ZERO, isPermanent: true },
    );
    f.answer(base, VOTING_ESCROW, IVotingEscrow::balanceOfNFTCall { tokenId: id }, U256::from(3 * E18 / 2));
    f.answer(base, VOTING_ESCROW, IVotingEscrow::delegatesCall { delegator: id }, U256::ZERO);
    f.answer(
        base,
        VOTER,
        IVoter::tokenStatesCall { tokenId: id },
        IVoter::tokenStatesReturn { committed: 3 * E18 / 2, lastStakeEnd: U48::ZERO, lastAllocated: U48::from(T0), isPermanent: true },
    );
    f.answer(base, VOTER, IVoter::allocationChainIdsCall { tokenId: id }, vec![U256::from(base), U256::from(10)]);
    f.answer(base, VOTER, IVoter::allocationChainAmountsCall { tokenId: id, chainId: U256::from(base) }, E18);
    f.answer(base, VOTER, IVoter::allocationChainAmountsCall { tokenId: id, chainId: U256::from(10) }, E18 / 2);

    for c in CHAINS {
        f.answer(c.id, LEAF_VOTER, ILeafVoter::allocationCooldownCall {}, U48::from(169_200));
        f.answer(c.id, LEAF_VOTER, ILeafVoter::maxAccumulatedCooldownReductionCall {}, U48::ZERO);
        f.answer(c.id, LEAF_VOTER, ILeafVoter::accumulatedCooldownReductionCall { tokenId: id }, U48::ZERO);
        let (last, alloc, gauges) = match c.id {
            8453 => (T0, E18, vec![G2, G1]),
            10 => (T0 + 600, E18 / 2, vec![G3]),
            _ => (0, 0, vec![]),
        };
        f.answer(
            c.id,
            LEAF_VOTER,
            ILeafVoter::tokenStatesCall { tokenId: id },
            ILeafVoter::tokenStatesReturn {
                operator: Address::ZERO,
                lastAllocated: U48::from(last),
                canVoteForZeroCapGauges: false,
                chainAllocation: alloc,
            },
        );
        f.answer(c.id, LEAF_VOTER, ILeafVoter::allocatedGaugesCall { tokenId: id }, gauges);
    }
    f.answer(base, LEAF_VOTER, ILeafVoter::allocationsCall { tokenId: id, gauge: G1 }, 3 * E18 / 4);
    f.answer(base, LEAF_VOTER, ILeafVoter::allocationsCall { tokenId: id, gauge: G2 }, E18 / 4);
    f.answer(10, LEAF_VOTER, ILeafVoter::allocationsCall { tokenId: id, gauge: G3 }, E18 / 2);

    f.answer(base, G1, IGauge::poolCall {}, P1);
    f.answer(base, G2, IGauge::poolCall {}, P2);
    f.answer(base, P1, IPool::token0Call {}, USDC);
    f.answer(base, P1, IPool::token1Call {}, POD);
    f.answer(base, P1, IPool::tickSpacingCall {}, I24::try_from(200).unwrap());
    f.answer(base, P1, IPool::feeCall {}, U24::from(10_500));
    f.answer(base, P2, IPool::token0Call {}, VOL);
    f.answer(base, P2, IPool::token1Call {}, USDC);
    f.answer(base, P2, IPool::stableCall {}, false);
    f.answer(base, USDC, IERC20::symbolCall {}, "USDC".to_string());
    f.answer(base, POD, IERC20::symbolCall {}, "POD".to_string());
    f.answer(base, VOL, IERC20::symbolCall {}, "VOL".to_string());
    f
}

fn no_env(_: &str) -> Option<String> {
    None
}

fn reader(f: FakeChains, env: &dyn Fn(&str) -> Option<String>) -> (Reader, Arc<FakeChains>) {
    let f = Arc::new(f);
    (Reader::new(Rpc::with_env(f.clone(), env)), f)
}

#[tokio::test]
async fn status_reports_details_cooldowns_and_votes() {
    let (r, f) = reader(world(&no_env), &no_env);
    let rep = r.report(Target::Wallet(WALLET), &Options::default(), T0 + H).await.unwrap();
    assert!(rep.complete, "{:?}", rep.errors);
    assert_eq!(rep.wallet.as_deref(), Some("0x63b4F15788aaA877a9466D2A5c1400325379931a"));
    let p = &rep.positions[0];
    assert_eq!(p.token_id, "4235");

    let d = p.details.as_ref().unwrap();
    assert_eq!(d.staked.value, "1.5");
    assert!(d.permanent && d.lock_end.is_none() && d.delegated_to.is_none());
    let alloc: Vec<_> = d.allocations.iter().map(|a| (a.chain, a.amount.value.as_str())).collect();
    assert_eq!(alloc, [("Base", "1"), ("Optimism", "0.5")]);

    // Only the chains it allocated on, in fixed order.
    let chains: Vec<_> = p.chains.iter().map(|c| c.chain).collect();
    assert_eq!(chains, ["Base", "Optimism"]);

    let base = &p.chains[0];
    let cd = base.cooldown.as_ref().unwrap();
    assert_eq!(cd.unlocks_at.unix, T0 + 169_200);
    assert_eq!(cd.unlocks_at.utc, "2026-10-24T14:21:37Z");
    assert_eq!(cd.seconds_left, 169_200 - H);
    assert!(!cd.ready);

    let votes = base.votes.as_ref().unwrap();
    assert_eq!(votes[0].pool_name.as_deref(), Some("USDC/POD"));
    assert_eq!((votes[0].pool_type, votes[0].tick_spacing, votes[0].fee_pct), (Some("concentrated"), Some(200), Some(1.05)));
    assert_eq!((votes[0].share_pct, votes[0].weight.value.as_str()), (75.0, "0.75"));
    assert_eq!((votes[1].pool_name.as_deref(), votes[1].pool_type, votes[1].fee_pct), (Some("VOL/USDC"), Some("volatile"), None));
    assert_eq!(votes[1].share_pct, 25.0);

    // A gauge whose pool can't be read still shows its weight.
    let op = &p.chains[1].votes.as_ref().unwrap()[0];
    assert_eq!((op.pool.as_deref(), op.pool_name.as_deref(), op.share_pct), (None, None, 100.0));

    // 2 (token list) + 2 (details) on Base, 7 cooldown reads, 3 more on Base to name the pools,
    // 1 on Optimism (its pool lookup finds nothing to name).
    assert_eq!(f.hits.lock().unwrap().len(), 2 + 2 + 7 + 3 + 1);
}

#[tokio::test]
async fn ready_once_the_cooldown_is_over_and_reduction_applies() {
    let mut f = world(&no_env);
    let id = U256::from(4235);
    f.answer(8453, LEAF_VOTER, ILeafVoter::maxAccumulatedCooldownReductionCall {}, U48::from(2 * H));
    f.answer(8453, LEAF_VOTER, ILeafVoter::accumulatedCooldownReductionCall { tokenId: id }, U48::from(5 * H)); // capped at 2h
    let (r, _) = reader(f, &no_env);
    let opts = Options { votes: false, details: false, ..Options::default() };
    let rep = r.report(Target::Tokens(vec![id]), &opts, T0 + 45 * H).await.unwrap();
    let base = rep.positions[0].chains[0].cooldown.as_ref().unwrap();
    assert_eq!((base.reduction_seconds, base.unlocks_at.unix, base.ready), (2 * H, T0 + 45 * H, true));
    assert_eq!(base.seconds_left, 0);
    let op = rep.positions[0].chains[1].cooldown.as_ref().unwrap();
    assert!(!op.ready, "Optimism has no reduction");
    assert!(rep.positions[0].chains.iter().all(|c| c.votes.is_none()));
    assert!(rep.positions[0].details.is_none());
}

#[tokio::test]
async fn a_chain_that_does_not_answer_is_reported_not_fatal() {
    let mut f = world(&no_env);
    f.fail_chain(10);
    let (r, _) = reader(f, &no_env);
    let rep = r.report(Target::Wallet(WALLET), &Options::default(), T0 + H).await.unwrap();
    assert!(!rep.complete);
    assert_eq!(rep.errors.len(), 1);
    assert_eq!(rep.errors[0].chain, Some("Optimism"));
    assert!(rep.errors[0].error.contains("HTTP 429"), "{}", rep.errors[0].error);
    let chains: Vec<_> = rep.positions[0].chains.iter().map(|c| c.chain).collect();
    assert_eq!(chains, ["Base"]);
}

#[tokio::test]
async fn retries_on_a_backup_rpc_and_says_so() {
    let mut f = world(&no_env);
    f.failing.insert("https://mainnet.base.org".into());
    let (r, f) = reader(f, &no_env);
    let rep = r.report(Target::Wallet(WALLET), &Options::default(), T0 + H).await.unwrap();
    assert!(rep.complete);
    assert!(rep.warnings.iter().all(|w| w.starts_with("Base: mainnet.base.org failed (HTTP 429)")), "{:?}", rep.warnings);
    // Every Base request needed its retry: 2 token list + 2 details + 1 cooldown + 3 votes.
    assert_eq!(rep.warnings.len(), 8);
    let hits = f.hits.lock().unwrap();
    assert!(hits.iter().any(|u| u == "https://base-rpc.publicnode.com") && hits.iter().any(|u| u == "https://base.drpc.org"));
}

#[tokio::test]
async fn alchemy_key_goes_first_and_never_shows_up_in_output() {
    let env = |k: &str| (k == "ALCHEMY_API_KEY").then(|| "testkey".to_string());
    let mut f = world(&env);
    f.failing.insert("https://base-mainnet.g.alchemy.com/v2/testkey".into());
    let (r, f) = reader(f, &env);
    let rep = r.report(Target::Wallet(WALLET), &Options::default(), T0 + H).await.unwrap();
    assert!(rep.complete);
    let json = serde_json::to_string(&rep).unwrap();
    assert!(json.contains("base-mainnet.g.alchemy.com failed"));
    assert!(!json.contains("testkey"));
    let hits = f.hits.lock().unwrap();
    assert!(hits.iter().any(|u| u == "https://opt-mainnet.g.alchemy.com/v2/testkey"));
}

#[tokio::test]
async fn empty_wallet_and_unknown_token() {
    let mut f = world(&no_env);
    let empty = address!("00000000000000000000000000000000000000aa");
    f.answer(8453, VOTING_ESCROW, IVotingEscrow::balanceOfCall { owner: empty }, U256::ZERO);
    let (r, _) = reader(f, &no_env);
    let rep = r.report(Target::Wallet(empty), &Options::default(), T0).await.unwrap();
    assert!(rep.complete && rep.positions.is_empty());

    // A balance that can't be read is fatal: there is nothing to report on.
    let unreadable = address!("00000000000000000000000000000000000000bb");
    assert!(r.report(Target::Wallet(unreadable), &Options::default(), T0).await.is_err());

    let rep = r.report(Target::Tokens(vec![U256::from(999_999)]), &Options::default(), T0).await.unwrap();
    assert!(!rep.complete);
    assert_eq!(rep.errors.iter().filter(|e| e.token_id.as_deref() == Some("999999")).count(), 1);
}

#[test]
fn amounts_and_durations() {
    assert_eq!(Amount::from_u128(1_105_804_177_318_636_222).value, "1.105804177318636222");
    assert_eq!(Amount::from_u128(2 * E18).value, "2");
    assert_eq!(Amount::from_u128(0).value, "0");
    assert_eq!(format_duration(0), "now");
    assert_eq!(format_duration(59), "1m");
    assert_eq!(format_duration(86_400 + 8 * 3600 + 26 * 60), "1d 8h 26m");
}

/// Real chains: the README's example wallet. `cargo test -- --ignored`.
#[tokio::test]
#[ignore]
async fn live_example_wallet() {
    let reader = Reader::new(Rpc::from_env(Arc::new(aero_pos::HttpTransport::new().unwrap())));
    let rep = reader.report(Target::Wallet(WALLET), &Options::default(), aero_pos::unix_now()).await.unwrap();
    let p = rep.positions.iter().find(|p| p.token_id == "4235").expect("#4235 in the wallet");
    let base = p.chains.iter().find(|c| c.chain_id == 8453).expect("#4235 allocated on Base");
    assert_eq!(base.cooldown.as_ref().unwrap().cooldown_seconds, 169_200);
    assert!(base.votes.as_ref().is_some_and(|v| !v.is_empty()));
}
