//! MimiEyes (VENDORED.md #3): a token-usage ledger.
//!
//! Every harness reports each turn's tokens (`AgentEvent::Usage`, emitted
//! once per turn before `Done`); stock Zeron drops them. MimiEyes appends one
//! line per turn to `hz-usage.jsonl` in the engine's data dir — when, which
//! chat on which device, the CLI and model the turn ran with, and its input /
//! output tokens — and serves the lines (`HzUsage`) to the window's Settings →
//! Usage page, which groups them. Projects come from the chat at read time;
//! CLI and model are stamped per turn (switching models mid-chat keeps both).

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError, RwLock};

use serde::{Deserialize, Serialize};
use zeron_proto::HarnessId;
use zeron_rpc::{RpcError, RpcReply};

/// The window's read of the ledger (params: `{ since?: epoch ms }`).
pub const HZ_USAGE: &str = "HzUsage";

const FILE: &str = "hz-usage.jsonl";

/// One turn's tokens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRecord {
    /// Epoch milliseconds.
    pub at: i64,
    pub chat_id: String,
    pub device_id: String,
    pub harness: HarnessId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// The ledger's file (the latest engine assembly wins: a profile switch moves
/// it with the data dir). `None` in bare tests: nothing is written.
static PATH: RwLock<Option<PathBuf>> = RwLock::new(None);
/// Appends never interleave.
static WRITE: Mutex<()> = Mutex::new(());

/// Point the ledger at `data_dir` (engine assembly).
pub fn init(data_dir: &std::path::Path) {
    *PATH.write().unwrap_or_else(PoisonError::into_inner) = Some(data_dir.join(FILE));
}

fn path() -> Option<PathBuf> {
    PATH.read().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Record one turn. Zero-token reports (a probe, an aborted start) are skipped.
pub fn record(
    chat_id: &str,
    device_id: &str,
    harness: HarnessId,
    model: Option<&str>,
    input_tokens: u64,
    output_tokens: u64,
) {
    if input_tokens == 0 && output_tokens == 0 {
        return;
    }
    let Some(path) = path() else {
        return;
    };
    let record = UsageRecord {
        at: chrono::Utc::now().timestamp_millis(),
        chat_id: chat_id.to_owned(),
        device_id: device_id.to_owned(),
        harness,
        model: model.map(str::to_owned).filter(|m| !m.trim().is_empty()),
        input_tokens,
        output_tokens,
    };
    let Ok(mut line) = serde_json::to_string(&record) else {
        return;
    };
    line.push('\n');
    let _guard = WRITE.lock().unwrap_or_else(PoisonError::into_inner);
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| file.write_all(line.as_bytes()));
    if let Err(err) = written {
        tracing::warn!(error = %err, "usage ledger write failed");
    }
}

/// Every record at or after `since` (epoch ms; `None` = all). Unreadable lines
/// are skipped, so a torn last line never hides the rest.
pub fn read(since: Option<i64>) -> Vec<UsageRecord> {
    let Some(path) = path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    parse(&text, since)
}

fn parse(text: &str, since: Option<i64>) -> Vec<UsageRecord> {
    text.lines()
        .filter_map(|line| serde_json::from_str::<UsageRecord>(line).ok())
        .filter(|r| since.is_none_or(|since| r.at >= since))
        .collect()
}

#[derive(Deserialize, Default)]
struct UsageParams {
    #[serde(default)]
    since: Option<i64>,
}

pub fn handles(method: &str) -> bool {
    method == HZ_USAGE
}

pub async fn handle(params: serde_json::Value) -> Result<RpcReply, RpcError> {
    let p: UsageParams = serde_json::from_value(params).unwrap_or_default();
    let records = tokio::task::spawn_blocking(move || read(p.since))
        .await
        .map_err(|e| RpcError::Failed(e.to_string()))?;
    RpcReply::value(&serde_json::json!({ "records": records }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_round_trip_and_filter_by_time() {
        let a = UsageRecord {
            at: 1_000,
            chat_id: "c1".into(),
            device_id: "d".into(),
            harness: HarnessId::ClaudeCode,
            model: Some("claude-opus-5-5".into()),
            input_tokens: 1200,
            output_tokens: 300,
        };
        let b = UsageRecord {
            at: 5_000,
            model: None,
            harness: HarnessId::Codex,
            ..a.clone()
        };
        let text = format!(
            "{}\n{{torn\n{}\n",
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
        assert_eq!(parse(&text, None), vec![a.clone(), b.clone()]);
        assert_eq!(parse(&text, Some(2_000)), vec![b]);
    }
}
