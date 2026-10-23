//! JSON-RPC `eth_call` over HTTP, Multicall3 batching, and one retry on a backup RPC.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use alloy_primitives::{Address, hex};
use alloy_sol_types::SolCall;
use anyhow::{Result, anyhow, bail};
use futures::future::BoxFuture;
use serde_json::json;

use crate::abi::{Call3, aggregate3Call};
use crate::config::{CALLS_PER_MULTICALL, CHAINS, Chain, MULTICALL3, RPC_TIMEOUT_SECS, rpc_urls};

/// Sends one `eth_call`. A trait so tests can swap in fake chains.
pub trait Transport: Send + Sync {
    fn eth_call<'a>(&'a self, url: &'a str, to: Address, data: Vec<u8>) -> BoxFuture<'a, Result<Vec<u8>>>;
}

pub struct HttpTransport {
    client: reqwest::Client,
}

impl HttpTransport {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(RPC_TIMEOUT_SECS))
            .user_agent(concat!("aero-pos/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { client })
    }
}

impl Transport for HttpTransport {
    fn eth_call<'a>(&'a self, url: &'a str, to: Address, data: Vec<u8>) -> BoxFuture<'a, Result<Vec<u8>>> {
        Box::pin(async move {
            let body = json!({
                "jsonrpc": "2.0", "id": 1, "method": "eth_call",
                "params": [{ "to": to.to_string(), "data": hex::encode_prefixed(&data) }, "latest"],
            });
            // without_url(): provider URLs carry API keys, keep them out of error messages.
            let res = self.client.post(url).json(&body).send().await.map_err(|e| anyhow!("{}", e.without_url()))?;
            if !res.status().is_success() {
                bail!("HTTP {}", res.status().as_u16());
            }
            let v: serde_json::Value = res.json().await.map_err(|e| anyhow!("{}", e.without_url()))?;
            if let Some(err) = v.get("error") {
                bail!("RPC error: {}", err.get("message").and_then(|m| m.as_str()).unwrap_or("unknown"));
            }
            let result = v.get("result").and_then(|r| r.as_str()).ok_or_else(|| anyhow!("empty result"))?;
            Ok(hex::decode(result)?)
        })
    }
}

/// Host only, for messages: provider URLs carry the API key in the path.
pub fn rpc_host(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "(bad RPC URL)".into())
}

pub struct Rpc {
    transport: Arc<dyn Transport>,
    urls: HashMap<u64, Vec<String>>,
    backup_turn: AtomicUsize,
    warnings: Mutex<Vec<String>>,
}

impl Rpc {
    /// RPC URLs come from the environment: RPC_<CHAIN>, ALCHEMY_API_KEY, then the public defaults.
    pub fn from_env(transport: Arc<dyn Transport>) -> Self {
        Self::with_env(transport, &|k| std::env::var(k).ok())
    }

    pub fn with_env(transport: Arc<dyn Transport>, env: &dyn Fn(&str) -> Option<String>) -> Self {
        let urls = CHAINS.iter().map(|c| (c.id, rpc_urls(c, env))).collect();
        Self { transport, urls, backup_turn: AtomicUsize::new(0), warnings: Mutex::new(Vec::new()) }
    }

    /// Recovered failures (a backup RPC answered), for the report.
    pub fn take_warnings(&self) -> Vec<String> {
        std::mem::take(&mut *self.warnings.lock().unwrap())
    }

    /// `eth_call` on the first-choice RPC, then once more on a backup if that fails.
    /// Backups rotate between failures so one bad backup does not keep losing the retry.
    pub async fn call(&self, chain: &Chain, to: Address, data: Vec<u8>) -> Result<Vec<u8>> {
        let urls = &self.urls[&chain.id];
        let first = rpc_host(&urls[0]);
        match self.transport.eth_call(&urls[0], to, data.clone()).await {
            Ok(v) => Ok(v),
            Err(e) if urls.len() > 1 => {
                let backup = &urls[1 + self.backup_turn.fetch_add(1, Ordering::Relaxed) % (urls.len() - 1)];
                let host = rpc_host(backup);
                match self.transport.eth_call(backup, to, data).await {
                    Ok(v) => {
                        self.warnings.lock().unwrap().push(format!("{}: {first} failed ({e}); {host} answered", chain.name));
                        Ok(v)
                    }
                    Err(e2) => Err(anyhow!("{first}: {e}; backup {host}: {e2}")),
                }
            }
            Err(e) => Err(anyhow!("{first}: {e}")),
        }
    }

    /// Runs calls through Multicall3.aggregate3 (allowFailure = true), chunked so one request
    /// never gets too large. `None` = that call reverted or returned nothing.
    pub async fn multicall(&self, chain: &Chain, calls: &[(Address, Vec<u8>)]) -> Result<Vec<Option<Vec<u8>>>> {
        let mut out = Vec::with_capacity(calls.len());
        for chunk in calls.chunks(CALLS_PER_MULTICALL) {
            let req = aggregate3Call {
                calls: chunk
                    .iter()
                    .map(|(target, data)| Call3 { target: *target, allowFailure: true, callData: data.clone().into() })
                    .collect(),
            };
            let raw = self.call(chain, MULTICALL3, req.abi_encode()).await?;
            let results = aggregate3Call::abi_decode_returns(&raw)?;
            if results.len() != chunk.len() {
                bail!("multicall returned {} results for {} calls", results.len(), chunk.len());
            }
            out.extend(results.into_iter().map(|r| (r.success && !r.returnData.is_empty()).then(|| r.returnData.to_vec())));
        }
        Ok(out)
    }
}
