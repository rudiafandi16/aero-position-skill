---
name: aero-positions
description: Check Aero (MetaDEX 03) sAERO positions for a wallet or token ids: stake, voting power, the 47h allocation cooldown on each Aero chain, and which pools each position votes for. Use when the user asks when they can reallocate or vote again on Aero, what their sAERO votes for, or how much AERO a position holds.
version: 0.1.0
metadata:
  hermes:
    tags: [defi, aero, base, voting, cooldown]
    category: defi
---

# Aero positions

Reads public on-chain data through the `aero-pos` CLI. Read-only: it never needs keys, signatures or transactions.

## When to Use

- "When can I reallocate / vote again on Aero?"
- "What does my sAERO position vote for?"
- "How much AERO is locked in position #4235?" / "Is it a permanent lock?"
- Any question about Aero allocation cooldowns on Base, Optimism, Arbitrum, Ethereum, Ink, Robinhood or Arc.

## Procedure

1. Get the target from the user: one wallet address (`0x…`) or token ids (`4235` or `4235,4245`). Ask if they gave neither.
2. Pick the command:
   - `status`: everything (default when unsure)
   - `cooldowns`: only "when can I reallocate"
   - `votes`: only "what does it vote for"
   - `positions`: only stake, voting power and lock (fastest, Base only)
3. Run it in the terminal, adding `--chains base,optimism` only if the user asked about specific chains:

   ```bash
   ~/.cargo/bin/aero-pos status <target> --compact
   ```

4. Parse stdout as JSON and answer from it (see below). Give times in UTC from the `utc` fields, plus "in X h Y m" from `seconds_left`.

## Reading the Result

- **Exit code 1**: stdout is `{"error": "..."}`. Tell the user what went wrong (usually a mistyped address).
- **`complete: false`**: some chain or position could not be read. Name it from `errors[]`, answer from the rest, and suggest trying again. Never guess missing data.
- **Can it reallocate on chain X?** Find `positions[].chains[]` with that `chain`:
  - if `cooldown.ready` is `true`, yes, now;
  - otherwise, at `cooldown.unlocks_at.utc`, which is `cooldown.seconds_left` seconds after `generated_at`.
- **A chain missing from `chains[]`**: the position never allocated there, so it has no cooldown there.
- **Cooldowns are per chain.** Reallocating on Base does not reset Optimism's countdown, and the reverse holds too.
- **Votes**: `chains[].votes[]` is sorted largest first. Report `pool_name` (fall back to `pool`, then `gauge`), `share_pct` and `weight.value` (AERO).
- **Amounts** are strings: use `value` (AERO) as written, and don't round it into a float.
- `warnings[]` only means a backup RPC answered. The data is still good, so don't mention it unless asked.

## Pitfalls

- A wallet only finds positions it **owns**. Positions held by a vault or another contract need that contract's address, or the token ids.
- `fee_pct` is the pool fee at read time. Many Aero pools use dynamic fees, so it can change.
- At most 50 positions per wallet are read.
- If the command is not found, the binary is not installed in this environment. See Verification.

## Verification

```bash
~/.cargo/bin/aero-pos --version
~/.cargo/bin/aero-pos status 0x63b4f15788aaa877a9466d2a5c1400325379931a --text
```

The second command should list positions #4235 and #4245 with their Base cooldowns. Free public RPCs are used by default. Set `ALCHEMY_API_KEY` (one key for all 7 chains) or `RPC_BASE`, `RPC_OPTIMISM`, … in the terminal environment for more reliable reads.
