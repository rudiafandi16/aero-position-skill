# aero-pos

A read-only Rust CLI and library that tracks Aero (MetaDEX 03) **sAERO positions** for agents and people:

- **Position details**: AERO staked, voting power, permanent lock or lock end, delegation
- **Cooldowns**: when each position can change its allocation again, on each of the 7 Aero chains
- **Votes**: which gauges (pools) each position votes for on each chain, with weight and share

It prints JSON by default, made for agents to call as a tool. It only reads public on-chain data: no keys, no signing, no transactions.

```bash
aero-pos status 0x63b4f15788aaa877a9466d2a5c1400325379931a --text
```
```
Wallet 0x63b4…931a · 2 positions · 2026-10-23 07:17 UTC

#4235  1.1058 AERO staked (permanent lock) · voting power 1.1058
  Base       allocated 1.1058 · cooldown ends in 1d 7h 4m (2026-10-24 14:21 UTC)
             100.00%  USDC/POD  (concentrated, tick 200, fee 1.05%)

#4245  0.0009 AERO staked (permanent lock) · voting power 0.0009
  Base       allocated 0.0009 · cooldown ends in 20h 34m (2026-10-24 03:52 UTC)
             100.00%  USDC/WHUF  (concentrated, tick 200, fee 0.326%)
```

## Install

You need Rust (stable). With Homebrew: `brew install rustup`, then `rustup default stable`. Homebrew's rustup is not on your PATH by default; add it once:

```bash
echo 'export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"' >> ~/.zshrc
```

Then build and install the `aero-pos` binary into `~/.cargo/bin`:

```bash
cargo install --path .
```

### RPCs

The tool works with no setup on public RPCs. A failed request is retried once on a backup public RPC. Public RPCs are rate-limited, so for regular use set one of these:

| Env var | Effect |
|---|---|
| `ALCHEMY_API_KEY` | One Alchemy key for all 7 chains. The Alchemy app needs Base, OP Mainnet, Arbitrum, Ethereum, Ink, Robinhood Chain and Arc enabled. |
| `RPC_BASE`, `RPC_OPTIMISM`, `RPC_ARBITRUM`, `RPC_ETHEREUM`, `RPC_INK`, `RPC_ROBINHOOD`, `RPC_ARC` | A full RPC URL for one chain. It comes before Alchemy and the public RPCs. |

The public RPCs stay on as backups. Output and errors only ever name the RPC **host**, never the full URL, so keys don't leak into agent transcripts.

## Commands

| Command | Reads | Use it for |
|---|---|---|
| `aero-pos status <target>` | everything | the default call |
| `aero-pos positions <target>` | Base only (fastest) | stake, voting power, per-chain allocation |
| `aero-pos cooldowns <target>` | all chains | "when can I reallocate?" |
| `aero-pos votes <target>` | all chains | "what does this position vote for?" |

`<target>` is one wallet address (`0x…`) or one or more token ids (`4235`, `#4235`, `4235,4245`). A wallet finds every position it **owns**, up to 50. Positions held by a vault or another contract belong to that contract, so query the contract or the token ids instead.

Options (any command): `--chains base,optimism` (keys: base, optimism, arbitrum, ethereum, ink, robinhood, arc), `--text` (for people), `--compact` (JSON on one line).

## For agents

Call the binary, read stdout as JSON, and follow these rules:

1. **Exit code 1** means bad input or the positions could not be read on Base. stdout is then `{"error": "..."}`.
2. **Exit code 0** means a report was printed. Check `complete`. If it's `false`, look at `errors`: one chain may not have answered while the rest is valid. Say which chain is missing instead of guessing.
3. **Can position X reallocate on chain Y?** Look at `positions[].chains[]` where `chain_id` = Y. If `cooldown.ready` is `true`, it can now. Otherwise it can at `cooldown.unlocks_at`, in `cooldown.seconds_left` seconds from `generated_at`. Recompute from `unlocks_at.unix` if time has passed since the call.
4. **A chain missing from `chains[]`** means the position never allocated there, so it has no cooldown there.
5. **Cooldowns are per chain.** Reallocating on Base does not reset Optimism's countdown.
6. **Amounts** come as `{raw, value}`: `raw` is a wei integer string, `value` is AERO as a decimal string. Don't use floats for money; `share_pct` and `fee_pct` are the only floats.
7. **Fees can change.** Many Aero pools use dynamic fees, so `fee_pct` is the fee at read time.

### JSON fields

```jsonc
{
  "generated_at": { "unix": 1792739838, "utc": "2026-10-23T07:17:18Z" },
  "wallet": "0x63b4F157…931a",          // null for token-id queries
  "complete": true,                      // false => see errors
  "positions": [{
    "token_id": "4235",
    "details": {                         // null for cooldowns/votes commands
      "owner": "0x…",
      "staked": { "raw": "1105804177318636222", "value": "1.105804177318636222" },
      "voting_power": { "raw": "…", "value": "…" },
      "permanent": true,
      "lock_end": null,                  // { unix, utc } for non-permanent locks
      "delegated_to": null,              // token id string if it delegates
      "committed": { "raw": "…", "value": "…" },   // hub Voter on Base
      "allocations": [{ "chain_id": 8453, "chain": "Base", "amount": { "raw": "…", "value": "…" } }]
    },
    "chains": [{                         // only chains it allocated on
      "chain_id": 8453, "chain": "Base",
      "allocated": { "raw": "…", "value": "…" },
      "cooldown": {                      // null with the votes command
        "last_allocated": { "unix": 1792682497, "utc": "2026-10-22T15:21:37Z" },
        "unlocks_at":     { "unix": 1792851697, "utc": "2026-10-24T14:21:37Z" },
        "seconds_left": 111859, "ready": false,
        "cooldown_seconds": 169200, "reduction_seconds": 0
      },
      "votes": [{                        // null with the cooldowns command; largest first
        "gauge": "0x1DD0…Ec6f", "pool": "0xc4D5…0f13",
        "pool_name": "USDC/POD",         // null if symbols could not be read
        "pool_type": "concentrated",     // or "stable" / "volatile"
        "tick_spacing": 200, "fee_pct": 1.05,
        "weight": { "raw": "…", "value": "…" },
        "share_pct": 100.0               // of this position's votes on this chain
      }]
    }]
  }],
  "errors": [{ "chain_id": 10, "chain": "Optimism", "token_id": null, "error": "…" }],
  "warnings": ["Base: mainnet.base.org failed (HTTP 429); base-rpc.publicnode.com answered"]
}
```

New fields may be added later, but existing ones won't be renamed.

### Example agent tool definition

Most frameworks can wrap the CLI as a tool like this:

```json
{
  "name": "aero_positions",
  "description": "Aero sAERO positions of a wallet or token ids: stake, voting power, the 47h allocation cooldown on each Aero chain (Base, Optimism, Arbitrum, Ethereum, Ink, Robinhood, Arc), and gauge votes. Read-only.",
  "input_schema": {
    "type": "object",
    "properties": {
      "command": { "enum": ["status", "positions", "cooldowns", "votes"] },
      "target": { "type": "string", "description": "Wallet 0x address, or token ids like \"4235,4245\"" },
      "chains": { "type": "string", "description": "Optional, e.g. \"base,optimism\"" }
    },
    "required": ["command", "target"]
  }
}
```

It runs as `aero-pos <command> <target> [--chains <chains>] --compact`.

## Library

```rust
use std::sync::Arc;
use aero_pos::{HttpTransport, Options, Reader, Rpc, Target};

let reader = Reader::new(Rpc::from_env(Arc::new(HttpTransport::new()?)));
let report = reader.report(Target::Wallet(wallet), &Options::default(), aero_pos::unix_now()).await?;
```

`Transport` is a trait, so you can plug in your own HTTP client, or a fake one for tests.

## How it works

Contracts verified on-chain on 2026-10-23 (sources on Sourcify):

| Contract | Address | Read |
|---|---|---|
| sAERO VotingEscrow (Base) | `0xbf7977e1521e9a3032bf7243ee4712f311f13915` | `balanceOf`, `tokenOfOwnerByIndex`, `ownerOf`, `staked`, `balanceOfNFT`, `delegates` |
| Voter, hub (Base) | `0x49d7d4f17ec56ef8fa04e464af5d1a4942cc31e6` | `tokenStates`, `allocationChainIds`, `allocationChainAmounts` |
| LeafVoter (all 7 chains) | `0x382409fad8503fbcdbb27a4211551fa03e884078` | `allocationCooldown`, `maxAccumulatedCooldownReduction`, `tokenStates`, `accumulatedCooldownReduction`, `allocatedGauges`, `allocations` |
| Gauge → pool → tokens | per vote | `pool()`, `token0/1()`, `tickSpacing()`, `fee()`, `stable()`, `symbol()` |

- Unlock time = `lastAllocated + allocationCooldown − min(accumulatedCooldownReduction, maxAccumulatedCooldownReduction)`. The cooldown is 169,200 s (47 h) on every chain today, and the reduction cap is 0.
- Every read goes through Multicall3. A `status` call on one wallet costs about 8 requests on Base plus 1 to 4 per other chain, whatever the number of positions.

## Develop

```bash
cargo test                 # offline: fake chains, RPC failures, backups, Alchemy key handling
cargo test -- --ignored    # also reads the real chains (example wallet above)
cargo clippy --all-targets
```

```
src/config.rs   contracts, chains, RPC order
src/abi.rs      contract functions (sol! definitions)
src/rpc.rs      eth_call, Multicall3, retry on a backup RPC
src/reader.rs   reads and assembles the report
src/report.rs   JSON report types
src/text.rs     --text rendering
src/main.rs     CLI
```
