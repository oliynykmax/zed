//! What the editor side of the infoview remembers between calls: notification
//! subscriptions, the latest server state to replay, open RPC sessions, and the
//! infoview's persistent state. Kept free of GPUI so it can be tested directly.

use anyhow::{Result, anyhow};
use collections::{HashMap, HashSet};
use lsp::LanguageServerId;
use project::lsp_store::lean_ext::{
    NotificationDirection, ObservedNotification, RpcKeepAliveParams, RpcSessionId,
};
use serde::Deserialize;
use serde_json::{Value, value::RawValue};

use crate::protocol::{InfoviewCall, StateContainer};

const PUBLISH_DIAGNOSTICS: &str = "textDocument/publishDiagnostics";
const FILE_PROGRESS: &str = "$/lean/fileProgress";
const DID_OPEN: &str = "textDocument/didOpen";
const DID_CLOSE: &str = "textDocument/didClose";

/// Server notifications whose latest value per document is replayed on subscription,
/// since the infoview may subscribe long after the server sent them.
const REPLAYED_METHODS: [&str; 2] = [PUBLISH_DIAGNOSTICS, FILE_PROGRESS];

#[derive(Debug, Clone, PartialEq)]
pub struct RpcSession {
    pub server: LanguageServerId,
    pub uri: lsp::Uri,
}

#[derive(Default)]
struct ServerTraffic {
    /// Keyed by method, then document URI.
    latest: HashMap<&'static str, HashMap<String, Box<RawValue>>>,
    open_documents: HashSet<String>,
}

#[derive(Default)]
pub struct HostState {
    subscriptions: HashMap<(NotificationDirection, String), usize>,
    servers: HashMap<LanguageServerId, ServerTraffic>,
    rpc_sessions: HashMap<RpcSessionId, RpcSession>,
    persistent_state: HashMap<(StateContainer, String), Value>,
}

impl HostState {
    /// Records a notification and returns the call that forwards it, if the infoview subscribed.
    pub fn observe(
        &mut self,
        server: LanguageServerId,
        notification: ObservedNotification,
    ) -> Option<InfoviewCall> {
        let traffic = self.servers.entry(server).or_default();
        let uri = document_uri(&notification.method, &notification.params);
        match (notification.direction, notification.method.as_str(), uri) {
            (NotificationDirection::FromServer, method, Some(uri)) => {
                if let Some(method) = REPLAYED_METHODS.into_iter().find(|m| *m == method) {
                    traffic
                        .latest
                        .entry(method)
                        .or_default()
                        .insert(uri, notification.params.clone());
                }
            }
            (NotificationDirection::FromClient, DID_OPEN, Some(uri)) => {
                traffic.open_documents.insert(uri);
            }
            (NotificationDirection::FromClient, DID_CLOSE, Some(uri)) => {
                traffic.open_documents.remove(&uri);
            }
            _ => {}
        }

        let key = (notification.direction, notification.method);
        if !self.subscriptions.contains_key(&key) {
            return None;
        }
        let (direction, method) = key;
        Some(match direction {
            NotificationDirection::FromServer => InfoviewCall::GotServerNotification {
                method,
                params: notification.params,
            },
            NotificationDirection::FromClient => InfoviewCall::SentClientNotification {
                method,
                params: notification.params,
            },
        })
    }

    /// Counts a subscription and returns the latest server values to replay to it.
    pub fn subscribe(
        &mut self,
        direction: NotificationDirection,
        method: String,
    ) -> Vec<InfoviewCall> {
        let replay = match direction {
            NotificationDirection::FromServer => self
                .servers
                .values()
                .filter_map(|traffic| traffic.latest.get(method.as_str()))
                .flat_map(|latest| latest.values())
                .map(|params| InfoviewCall::GotServerNotification {
                    method: method.clone(),
                    params: params.clone(),
                })
                .collect(),
            NotificationDirection::FromClient => Vec::new(),
        };
        *self.subscriptions.entry((direction, method)).or_default() += 1;
        replay
    }

    pub fn unsubscribe(&mut self, direction: NotificationDirection, method: String) -> Result<()> {
        let key = (direction, method);
        let count = self
            .subscriptions
            .get_mut(&key)
            .ok_or_else(|| anyhow!("no subscription to {:?} notifications {}", key.0, key.1))?;
        *count -= 1;
        if *count == 0 {
            self.subscriptions.remove(&key);
        }
        Ok(())
    }

    /// Forgets everything tied to a page that went away.
    pub fn page_reloaded(&mut self) {
        self.subscriptions.clear();
        self.rpc_sessions.clear();
        self.persistent_state
            .retain(|(container, _), _| *container != StateContainer::Infoview);
    }

    /// Returns the server that has the document open, if any has.
    pub fn server_for_document(&self, uri: &str) -> Option<LanguageServerId> {
        self.servers
            .iter()
            .find(|(_, traffic)| traffic.open_documents.contains(uri))
            .map(|(server, _)| *server)
    }

    pub fn server_removed(&mut self, server: LanguageServerId) {
        self.servers.remove(&server);
        self.rpc_sessions
            .retain(|_, session| session.server != server);
    }

    pub fn open_rpc_session(&mut self, id: RpcSessionId, session: RpcSession) {
        self.rpc_sessions.insert(id, session);
    }

    pub fn close_rpc_session(&mut self, id: &RpcSessionId) -> Option<RpcSession> {
        self.rpc_sessions.remove(id)
    }

    pub fn keep_alives(&self) -> Vec<(LanguageServerId, RpcKeepAliveParams)> {
        self.rpc_sessions
            .iter()
            .map(|(id, session)| {
                (
                    session.server,
                    RpcKeepAliveParams {
                        uri: session.uri.clone(),
                        session_id: id.clone(),
                    },
                )
            })
            .collect()
    }

    /// Returns the stored entry as `{ value }`, or null when there is none.
    pub fn read_state(&self, container: StateContainer, key: String) -> Value {
        match self.persistent_state.get(&(container, key)) {
            Some(value) => serde_json::json!({ "value": value }),
            None => Value::Null,
        }
    }

    pub fn save_state(&mut self, container: StateContainer, key: String, value: Value) {
        self.persistent_state.insert((container, key), value);
    }

    pub fn delete_state(&mut self, container: StateContainer, key: String) {
        self.persistent_state.remove(&(container, key));
    }
}

fn document_uri(method: &str, params: &RawValue) -> Option<String> {
    #[derive(Deserialize)]
    struct Document {
        uri: String,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct InDocument {
        text_document: Document,
    }

    match method {
        PUBLISH_DIAGNOSTICS => serde_json::from_str::<Document>(params.get())
            .ok()
            .map(|document| document.uri),
        FILE_PROGRESS | DID_OPEN | DID_CLOSE => serde_json::from_str::<InDocument>(params.get())
            .ok()
            .map(|params| params.text_document.uri),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const A: &str = "file:///p/A.lean";
    const B: &str = "file:///p/B.lean";

    fn notification(
        direction: NotificationDirection,
        method: &str,
        params: Value,
    ) -> ObservedNotification {
        ObservedNotification {
            direction,
            method: method.into(),
            params: RawValue::from_string(params.to_string()).unwrap(),
        }
    }

    fn diagnostics(uri: &str, message: &str) -> ObservedNotification {
        notification(
            NotificationDirection::FromServer,
            PUBLISH_DIAGNOSTICS,
            json!({"uri": uri, "diagnostics": [{"message": message}]}),
        )
    }

    fn scripts(calls: impl IntoIterator<Item = InfoviewCall>) -> Vec<String> {
        let mut scripts: Vec<_> = calls
            .into_iter()
            .map(|call| call.script().unwrap())
            .collect();
        scripts.sort();
        scripts
    }

    #[test]
    fn forwards_only_subscribed_notifications() {
        let mut host = HostState::default();
        let server = LanguageServerId(1);
        assert!(host.observe(server, diagnostics(A, "early")).is_none());

        host.subscribe(
            NotificationDirection::FromServer,
            PUBLISH_DIAGNOSTICS.into(),
        );
        host.subscribe(
            NotificationDirection::FromServer,
            PUBLISH_DIAGNOSTICS.into(),
        );
        let forwarded = host.observe(server, diagnostics(A, "late")).unwrap();
        assert_eq!(
            forwarded.script().unwrap(),
            r#"window.zedInfoview.call("gotServerNotification",["textDocument/publishDiagnostics",{"uri":"file:///p/A.lean","diagnostics":[{"message":"late"}]}])"#
        );

        host.unsubscribe(
            NotificationDirection::FromServer,
            PUBLISH_DIAGNOSTICS.into(),
        )
        .unwrap();
        assert!(host.observe(server, diagnostics(A, "still")).is_some());
        host.unsubscribe(
            NotificationDirection::FromServer,
            PUBLISH_DIAGNOSTICS.into(),
        )
        .unwrap();
        assert!(host.observe(server, diagnostics(A, "gone")).is_none());
        assert!(
            host.unsubscribe(
                NotificationDirection::FromServer,
                PUBLISH_DIAGNOSTICS.into()
            )
            .is_err()
        );
    }

    #[test]
    fn forwards_client_notifications_separately() {
        let mut host = HostState::default();
        let server = LanguageServerId(1);
        let did_close = || {
            notification(
                NotificationDirection::FromClient,
                DID_CLOSE,
                json!({"textDocument": {"uri": A}}),
            )
        };
        host.subscribe(NotificationDirection::FromServer, DID_CLOSE.into());
        assert!(host.observe(server, did_close()).is_none());
        host.subscribe(NotificationDirection::FromClient, DID_CLOSE.into());
        assert_eq!(
            host.observe(server, did_close()).unwrap().script().unwrap(),
            r#"window.zedInfoview.call("sentClientNotification",["textDocument/didClose",{"textDocument":{"uri":"file:///p/A.lean"}}])"#
        );
    }

    #[test]
    fn replays_latest_values_per_document_on_every_subscription() {
        let mut host = HostState::default();
        host.observe(LanguageServerId(1), diagnostics(A, "old"));
        host.observe(LanguageServerId(1), diagnostics(A, "new"));
        host.observe(LanguageServerId(2), diagnostics(B, "other server"));
        host.observe(
            LanguageServerId(1),
            notification(
                NotificationDirection::FromServer,
                FILE_PROGRESS,
                json!({"textDocument": {"uri": A, "version": 1}, "processing": []}),
            ),
        );

        let expected = vec![
            r#"window.zedInfoview.call("gotServerNotification",["textDocument/publishDiagnostics",{"uri":"file:///p/A.lean","diagnostics":[{"message":"new"}]}])"#.to_string(),
            r#"window.zedInfoview.call("gotServerNotification",["textDocument/publishDiagnostics",{"uri":"file:///p/B.lean","diagnostics":[{"message":"other server"}]}])"#.to_string(),
        ];
        assert_eq!(
            scripts(host.subscribe(
                NotificationDirection::FromServer,
                PUBLISH_DIAGNOSTICS.into()
            )),
            expected
        );
        assert_eq!(
            scripts(host.subscribe(
                NotificationDirection::FromServer,
                PUBLISH_DIAGNOSTICS.into()
            )),
            expected
        );
        assert_eq!(
            scripts(host.subscribe(NotificationDirection::FromServer, FILE_PROGRESS.into())),
            vec![
                r#"window.zedInfoview.call("gotServerNotification",["$/lean/fileProgress",{"textDocument":{"uri":"file:///p/A.lean","version":1},"processing":[]}])"#
            ]
        );
        assert!(
            host.subscribe(NotificationDirection::FromClient, DID_CLOSE.into())
                .is_empty()
        );

        host.server_removed(LanguageServerId(2));
        assert_eq!(
            scripts(host.subscribe(
                NotificationDirection::FromServer,
                PUBLISH_DIAGNOSTICS.into()
            )),
            expected[..1]
        );
    }

    #[test]
    fn routes_documents_to_the_server_that_opened_them() {
        let mut host = HostState::default();
        let open = |uri: &str| {
            notification(
                NotificationDirection::FromClient,
                DID_OPEN,
                json!({"textDocument": {"uri": uri, "languageId": "lean4", "version": 0, "text": ""}}),
            )
        };
        host.observe(LanguageServerId(1), open(A));
        host.observe(LanguageServerId(2), open(B));
        assert_eq!(host.server_for_document(A), Some(LanguageServerId(1)));
        assert_eq!(host.server_for_document(B), Some(LanguageServerId(2)));

        host.observe(
            LanguageServerId(1),
            notification(
                NotificationDirection::FromClient,
                DID_CLOSE,
                json!({"textDocument": {"uri": A}}),
            ),
        );
        assert_eq!(host.server_for_document(A), None);
    }

    #[test]
    fn keeps_alive_open_sessions_until_closed_or_server_removed() {
        let mut host = HostState::default();
        let a: lsp::Uri = A.parse().unwrap();
        let b: lsp::Uri = B.parse().unwrap();
        host.open_rpc_session(
            RpcSessionId("1".into()),
            RpcSession {
                server: LanguageServerId(1),
                uri: a.clone(),
            },
        );
        host.open_rpc_session(
            RpcSessionId("2".into()),
            RpcSession {
                server: LanguageServerId(2),
                uri: b.clone(),
            },
        );
        host.open_rpc_session(
            RpcSessionId("3".into()),
            RpcSession {
                server: LanguageServerId(2),
                uri: b.clone(),
            },
        );

        let mut keep_alives = host.keep_alives();
        keep_alives.sort_by(|x, y| x.1.session_id.0.cmp(&y.1.session_id.0));
        assert_eq!(
            keep_alives,
            vec![
                (
                    LanguageServerId(1),
                    RpcKeepAliveParams {
                        uri: a.clone(),
                        session_id: RpcSessionId("1".into())
                    }
                ),
                (
                    LanguageServerId(2),
                    RpcKeepAliveParams {
                        uri: b.clone(),
                        session_id: RpcSessionId("2".into())
                    }
                ),
                (
                    LanguageServerId(2),
                    RpcKeepAliveParams {
                        uri: b,
                        session_id: RpcSessionId("3".into())
                    }
                ),
            ]
        );

        assert_eq!(
            host.close_rpc_session(&RpcSessionId("3".into())),
            Some(RpcSession {
                server: LanguageServerId(2),
                uri: "file:///p/B.lean".parse().unwrap()
            })
        );
        host.server_removed(LanguageServerId(2));
        assert_eq!(
            host.keep_alives(),
            vec![(
                LanguageServerId(1),
                RpcKeepAliveParams {
                    uri: a,
                    session_id: RpcSessionId("1".into())
                }
            )]
        );
        host.page_reloaded();
        assert_eq!(host.keep_alives(), vec![]);
    }

    #[test]
    fn keeps_workspace_state_across_page_reloads() {
        let mut host = HostState::default();
        host.save_state(StateContainer::Infoview, "k".into(), json!(1));
        host.save_state(StateContainer::Workspace, "k".into(), json!(2));
        assert_eq!(
            host.read_state(StateContainer::Infoview, "k".into()),
            json!({"value": 1})
        );
        host.page_reloaded();
        assert_eq!(
            host.read_state(StateContainer::Infoview, "k".into()),
            Value::Null
        );
        assert_eq!(
            host.read_state(StateContainer::Workspace, "k".into()),
            json!({"value": 2})
        );
        host.delete_state(StateContainer::Workspace, "k".into());
        assert_eq!(
            host.read_state(StateContainer::Workspace, "k".into()),
            Value::Null
        );
    }
}
