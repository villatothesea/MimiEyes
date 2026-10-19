//! MimiEyes (VENDORED.md #3): a request/reply relay from agents to the
//! window. Panes and tabs are the window's own state — the engine never
//! stores them — so an agent's `panes` tool (Zeron MCP) asks the engine,
//! the engine hands the request to every window watching
//! [`WATCH_HZ_REQUESTS`], and the first window's [`HZ_REPLY`] is the
//! agent's answer. No window open (or none answering in time) is an error
//! the agent reads.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures::StreamExt as _;
use futures::stream::BoxStream;
use serde::Deserialize;
use tokio::sync::{broadcast, oneshot};
use zeron_rpc::{RpcError, RpcReply, parse_params};

/// An agent's request for the window (params: `{ from, request }`).
pub const HZ_REQUEST: &str = "HzRequest";
/// The window's stream of requests to serve.
pub const WATCH_HZ_REQUESTS: &str = "WatchHzRequests";
/// The window's answer (params: `{ id, result }` or `{ id, error }`).
pub const HZ_REPLY: &str = "HzReply";

/// Advertised in `EngineInfo.capabilities`: windows subscribe to the relay
/// only on engines that have it.
pub const HZ_RELAY_CAPABILITY: &str = "mimieyes.hzRelay.v1";

/// How long an agent waits for the window to answer.
const REPLY_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Default)]
pub struct HzRelay {
    inner: Arc<Inner>,
}

struct Inner {
    requests: broadcast::Sender<serde_json::Value>,
    pending: Mutex<HashMap<String, oneshot::Sender<Result<serde_json::Value, String>>>>,
    watchers: AtomicUsize,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            requests: broadcast::channel(64).0,
            pending: Mutex::new(HashMap::new()),
            watchers: AtomicUsize::new(0),
        }
    }
}

#[derive(Deserialize)]
struct ReplyParams {
    id: String,
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(default)]
    error: Option<String>,
}

/// Counts a window's stream while it lives.
struct Watching(Arc<Inner>);

impl Drop for Watching {
    fn drop(&mut self) {
        self.0.watchers.fetch_sub(1, Ordering::Relaxed);
    }
}

impl HzRelay {
    pub fn handles(method: &str) -> bool {
        matches!(method, HZ_REQUEST | WATCH_HZ_REQUESTS | HZ_REPLY)
    }

    pub async fn handle(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<RpcReply, RpcError> {
        match method {
            HZ_REQUEST => self.request(params).await.map(RpcReply::Value),
            WATCH_HZ_REQUESTS => Ok(RpcReply::Stream(self.watch())),
            HZ_REPLY => {
                let p: ReplyParams = parse_params(params)?;
                self.reply(p);
                RpcReply::value(&serde_json::json!({ "ok": true }))
            }
            _ => Err(RpcError::UnknownMethod(method.to_string())),
        }
    }

    /// Forward to the window(s) and wait for the first answer.
    async fn request(&self, params: serde_json::Value) -> Result<serde_json::Value, RpcError> {
        if self.inner.watchers.load(Ordering::Relaxed) == 0 {
            return Err(RpcError::Failed(
                "No MimiEyes window is open to arrange panes; ask the user to open MimiEyes."
                    .into(),
            ));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(id.clone(), tx);
        let sent = self
            .inner
            .requests
            .send(serde_json::json!({ "id": id, "params": params }));
        if sent.is_err() {
            self.inner.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&id);
            return Err(RpcError::Failed("No MimiEyes window is listening".into()));
        }
        let answer = tokio::time::timeout(REPLY_TIMEOUT, rx).await;
        self.inner.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&id);
        match answer {
            Ok(Ok(Ok(result))) => Ok(result),
            Ok(Ok(Err(error))) => Err(RpcError::Failed(error)),
            Ok(Err(_)) => Err(RpcError::Failed("The window closed before answering".into())),
            Err(_) => Err(RpcError::Failed(
                "The MimiEyes window did not answer in time".into(),
            )),
        }
    }

    fn reply(&self, p: ReplyParams) {
        if let Some(tx) = self.inner.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&p.id) {
            let answer = match (p.result, p.error) {
                (_, Some(error)) => Err(error),
                (Some(result), None) => Ok(result),
                (None, None) => Ok(serde_json::Value::Null),
            };
            let _ = tx.send(answer);
        }
    }

    /// A window's stream of requests; it counts as listening while open.
    fn watch(&self) -> BoxStream<'static, serde_json::Value> {
        self.inner.watchers.fetch_add(1, Ordering::Relaxed);
        let watching = Watching(self.inner.clone());
        let rx = self.inner.requests.subscribe();
        futures::stream::unfold((rx, watching), |(mut rx, watching)| async move {
            loop {
                match rx.recv().await {
                    Ok(item) => return Some((item, (rx, watching))),
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        })
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_request_without_a_window_fails_at_once() {
        let relay = HzRelay::default();
        let err = relay
            .handle(HZ_REQUEST, serde_json::json!({ "request": {} }))
            .await
            .err()
            .expect("no window");
        assert!(err.to_string().contains("No MimiEyes window"), "{err}");
    }

    #[tokio::test]
    async fn the_windows_reply_is_the_agents_answer() {
        let relay = HzRelay::default();
        let RpcReply::Stream(mut requests) =
            relay.handle(WATCH_HZ_REQUESTS, serde_json::Value::Null).await.unwrap()
        else {
            panic!("stream");
        };
        let asking = {
            let relay = relay.clone();
            tokio::spawn(async move {
                relay
                    .handle(HZ_REQUEST, serde_json::json!({ "request": { "op": "list" } }))
                    .await
            })
        };
        let item = requests.next().await.expect("a request");
        assert_eq!(item["params"]["request"]["op"], "list");
        let id = item["id"].as_str().unwrap().to_owned();
        relay
            .handle(HZ_REPLY, serde_json::json!({ "id": id, "result": { "tabs": [] } }))
            .await
            .unwrap();
        let RpcReply::Value(answer) = asking.await.unwrap().unwrap() else {
            panic!("value");
        };
        assert_eq!(answer, serde_json::json!({ "tabs": [] }));
    }

    #[tokio::test]
    async fn a_window_error_reaches_the_agent() {
        let relay = HzRelay::default();
        let RpcReply::Stream(mut requests) =
            relay.handle(WATCH_HZ_REQUESTS, serde_json::Value::Null).await.unwrap()
        else {
            panic!("stream");
        };
        let asking = {
            let relay = relay.clone();
            tokio::spawn(async move { relay.handle(HZ_REQUEST, serde_json::json!({})).await })
        };
        let id = requests.next().await.unwrap()["id"].as_str().unwrap().to_owned();
        relay
            .handle(HZ_REPLY, serde_json::json!({ "id": id, "error": "unknown chat" }))
            .await
            .unwrap();
        let err = asking.await.unwrap().err().expect("error");
        assert!(err.to_string().contains("unknown chat"));
    }
}
