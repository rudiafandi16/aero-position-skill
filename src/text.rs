//! Human-readable rendering of a [`Report`] (`--text`).

use std::fmt::Write;

use crate::report::{Amount, Report, Time, format_duration};

/// "1.105804177318636222" -> "1.1058".
fn short(a: &Amount) -> String {
    match a.value.split_once('.') {
        Some((int, frac)) if frac.len() > 4 => {
            let frac = frac[..4].trim_end_matches('0');
            if frac.is_empty() { int.to_string() } else { format!("{int}.{frac}") }
        }
        _ => a.value.clone(),
    }
}

/// "2026-10-24T14:21:37Z" -> "2026-10-24 14:21 UTC".
fn when(t: &Time) -> String {
    match t.utc.get(..16) {
        Some(s) => format!("{} UTC", s.replace('T', " ")),
        None => t.unix.to_string(),
    }
}

fn short_addr(a: &str) -> String {
    if a.len() > 10 { format!("{}…{}", &a[..6], &a[a.len() - 4..]) } else { a.to_string() }
}

pub fn render(r: &Report) -> String {
    let mut s = String::new();
    let n = r.positions.len();
    match &r.wallet {
        Some(w) => writeln!(s, "Wallet {} · {n} position{} · {}", short_addr(w), if n == 1 { "" } else { "s" }, when(&r.generated_at)),
        None => writeln!(s, "{n} position{} · {}", if n == 1 { "" } else { "s" }, when(&r.generated_at)),
    }
    .ok();
    if n == 0 {
        writeln!(s, "\nNo sAERO positions owned by this wallet.").ok();
    }

    for p in &r.positions {
        writeln!(s).ok();
        match &p.details {
            Some(d) => {
                let lock = if d.permanent {
                    "permanent lock".to_string()
                } else {
                    d.lock_end.as_ref().map(|t| format!("lock ends {}", when(t))).unwrap_or_else(|| "lock expired".into())
                };
                writeln!(s, "#{}  {} AERO staked ({lock}) · voting power {}", p.token_id, short(&d.staked), short(&d.voting_power)).ok();
                if let Some(to) = &d.delegated_to {
                    writeln!(s, "  delegates its votes to #{to}").ok();
                }
            }
            None => {
                writeln!(s, "#{}", p.token_id).ok();
            }
        }
        if p.chains.is_empty() {
            writeln!(s, "  not allocated on any chain").ok();
        }
        for c in &p.chains {
            let cooldown = match &c.cooldown {
                Some(cd) if cd.ready => "can reallocate now".to_string(),
                Some(cd) => format!("cooldown ends in {} ({})", format_duration(cd.seconds_left), when(&cd.unlocks_at)),
                None => "no cooldown".to_string(),
            };
            writeln!(s, "  {:<10} allocated {} · {cooldown}", c.chain, short(&c.allocated)).ok();
            for v in c.votes.iter().flatten() {
                let name = v.pool_name.clone().unwrap_or_else(|| short_addr(v.pool.as_deref().unwrap_or(&v.gauge)));
                let mut kind = v.pool_type.unwrap_or("pool").to_string();
                if let Some(ts) = v.tick_spacing {
                    write!(kind, ", tick {ts}").ok();
                }
                if let Some(fee) = v.fee_pct {
                    write!(kind, ", fee {fee}%").ok();
                }
                writeln!(s, "             {:>6.2}%  {name}  ({kind})", v.share_pct).ok();
            }
        }
    }

    if !r.errors.is_empty() {
        writeln!(s, "\nNot complete:").ok();
        for e in &r.errors {
            let who = e.chain.map(str::to_string).or_else(|| e.token_id.as_ref().map(|t| format!("#{t}"))).unwrap_or_default();
            writeln!(s, "  {who}: {}", e.error).ok();
        }
    }
    s
}
