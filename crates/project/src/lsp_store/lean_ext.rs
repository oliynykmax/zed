use std::time::Duration;

use ::serde::{Deserialize, Serialize};
use futures::{FutureExt as _, future::LocalBoxFuture};
use lsp::{IoKind, LanguageServer, LanguageServerName};
use serde_json::{Value, value::RawValue};

pub const LEAN_SERVER_NAME: LanguageServerName = LanguageServerName::new_static("lean4-lsp");

/// Returns the tactic state at a position, rendered as Markdown.
///
/// [Lean 4 server source](https://github.com/leanprover/lean4/blob/master/src/Lean/Server/FileWorker/RequestHandling.lean)
pub enum PlainGoal {}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlainGoalResult {
    pub rendered: String,
    pub goals: Vec<String>,
}

impl lsp::request::Request for PlainGoal {
    type Params = lsp::TextDocumentPositionParams;
    type Result = Option<PlainGoalResult>;
    const METHOD: &'static str = "$/lean/plainGoal";
}

/// Identifies an RPC session, which the server encodes as a decimal string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct RpcSessionId(pub String);

/// Opens an RPC session for the widgets of one document.
pub enum RpcConnect {}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RpcConnectParams {
    pub uri: lsp::Uri,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcConnected {
    pub session_id: RpcSessionId,
}

impl lsp::request::Request for RpcConnect {
    type Params = RpcConnectParams;
    type Result = RpcConnected;
    const METHOD: &'static str = "$/lean/rpc/connect";
}

/// Keeps an RPC session alive. The server closes sessions that stay quiet for about a minute.
pub enum RpcKeepAlive {}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcKeepAliveParams {
    pub uri: lsp::Uri,
    pub session_id: RpcSessionId,
}

impl lsp::notification::Notification for RpcKeepAlive {
    type Params = RpcKeepAliveParams;
    const METHOD: &'static str = "$/lean/rpc/keepAlive";
}

/// A request that the Lean infoview sends through the editor, with untyped params.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoviewRequest {
    RpcCall,
    PlainGoal,
    PlainTermGoal,
}

/// A notification that the Lean infoview sends through the editor, with untyped params.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoviewNotification {
    RpcRelease,
}

mod untyped {
    use serde_json::Value;

    pub enum RpcCall {}
    impl lsp::request::Request for RpcCall {
        type Params = Value;
        type Result = Value;
        const METHOD: &'static str = "$/lean/rpc/call";
    }

    pub enum PlainGoal {}
    impl lsp::request::Request for PlainGoal {
        type Params = Value;
        type Result = Value;
        const METHOD: &'static str = "$/lean/plainGoal";
    }

    pub enum PlainTermGoal {}
    impl lsp::request::Request for PlainTermGoal {
        type Params = Value;
        type Result = Value;
        const METHOD: &'static str = "$/lean/plainTermGoal";
    }

    pub enum RpcRelease {}
    impl lsp::notification::Notification for RpcRelease {
        type Params = Value;
        const METHOD: &'static str = "$/lean/rpc/release";
    }
}

impl InfoviewRequest {
    pub fn from_method(method: &str) -> Option<Self> {
        use lsp::request::Request as _;
        match method {
            untyped::RpcCall::METHOD => Some(Self::RpcCall),
            untyped::PlainGoal::METHOD => Some(Self::PlainGoal),
            untyped::PlainTermGoal::METHOD => Some(Self::PlainTermGoal),
            _ => None,
        }
    }

    /// Sends the request. Dropping the future cancels it on the server.
    pub fn send(
        self,
        server: &LanguageServer,
        params: Value,
        timeout: Duration,
    ) -> LocalBoxFuture<'static, anyhow::Result<Value>> {
        match self {
            Self::RpcCall => server
                .request::<untyped::RpcCall>(params, timeout)
                .map(|result| result.into_response())
                .boxed_local(),
            Self::PlainGoal => server
                .request::<untyped::PlainGoal>(params, timeout)
                .map(|result| result.into_response())
                .boxed_local(),
            Self::PlainTermGoal => server
                .request::<untyped::PlainTermGoal>(params, timeout)
                .map(|result| result.into_response())
                .boxed_local(),
        }
    }
}

impl InfoviewNotification {
    pub fn from_method(method: &str) -> Option<Self> {
        use lsp::notification::Notification as _;
        match method {
            untyped::RpcRelease::METHOD => Some(Self::RpcRelease),
            _ => None,
        }
    }

    pub fn send(self, server: &LanguageServer, params: Value) -> anyhow::Result<()> {
        match self {
            Self::RpcRelease => server.notify::<untyped::RpcRelease>(params),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotificationDirection {
    FromServer,
    FromClient,
}

/// A notification seen on a language server's stdio, with its params kept as raw JSON.
#[derive(Debug, Clone)]
pub struct ObservedNotification {
    pub direction: NotificationDirection,
    pub method: String,
    pub params: Box<RawValue>,
}

/// Parses one stdio message, keeping it only if it is a notification.
pub fn parse_notification(kind: IoKind, message: &str) -> Option<ObservedNotification> {
    #[derive(Deserialize)]
    struct Message<'a> {
        #[serde(default)]
        id: Option<&'a RawValue>,
        #[serde(borrow)]
        method: Option<std::borrow::Cow<'a, str>>,
        #[serde(borrow)]
        params: Option<&'a RawValue>,
    }

    let direction = match kind {
        IoKind::StdOut => NotificationDirection::FromServer,
        IoKind::StdIn => NotificationDirection::FromClient,
        IoKind::StdErr => return None,
    };
    let message: Message = serde_json::from_str(message).ok()?;
    if message.id.is_some() {
        return None;
    }
    let params = match message.params {
        Some(params) => params.to_owned(),
        None => RawValue::from_string("null".to_owned()).ok()?,
    };
    Some(ObservedNotification {
        direction,
        method: message.method?.into_owned(),
        params,
    })
}

/// Calls `f` with each notification the server sends or receives. Zed's own handlers still run.
#[must_use]
pub fn observe_notifications(
    server: &LanguageServer,
    mut f: impl FnMut(ObservedNotification) + Send + 'static,
) -> lsp::Subscription {
    server.on_io(move |kind, message| {
        if let Some(notification) = parse_notification(kind, message) {
            f(notification);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_notifications_in_both_directions() {
        let incoming = parse_notification(
            IoKind::StdOut,
            r#"{"jsonrpc":"2.0","method":"$/lean/fileProgress","params":{"processing":[]}}"#,
        )
        .unwrap();
        assert_eq!(incoming.direction, NotificationDirection::FromServer);
        assert_eq!(incoming.method, "$/lean/fileProgress");
        assert_eq!(incoming.params.get(), r#"{"processing":[]}"#);

        let outgoing = parse_notification(
            IoKind::StdIn,
            r#"{"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":"file:///a.lean"}}}"#,
        )
        .unwrap();
        assert_eq!(outgoing.direction, NotificationDirection::FromClient);
        assert_eq!(outgoing.method, "textDocument/didClose");
        assert_eq!(
            outgoing.params.get(),
            r#"{"textDocument":{"uri":"file:///a.lean"}}"#
        );
    }

    #[test]
    fn skips_requests_responses_and_stderr() {
        let request = r#"{"jsonrpc":"2.0","id":3,"method":"workspace/inlayHint/refresh"}"#;
        assert!(parse_notification(IoKind::StdOut, request).is_none());
        let response = r#"{"jsonrpc":"2.0","id":1,"result":{"sessionId":"120"}}"#;
        assert!(parse_notification(IoKind::StdOut, response).is_none());
        let notification = r#"{"jsonrpc":"2.0","method":"initialized"}"#;
        assert!(parse_notification(IoKind::StdErr, notification).is_none());
        assert_eq!(
            parse_notification(IoKind::StdIn, notification)
                .unwrap()
                .params
                .get(),
            "null"
        );
    }

    #[test]
    fn parses_escaped_method_names() {
        let message = r#"{"jsonrpc":"2.0","method":"$\/lean\/fileProgress","params":{}}"#;
        assert_eq!(
            parse_notification(IoKind::StdOut, message).unwrap().method,
            "$/lean/fileProgress"
        );
    }

    #[test]
    fn maps_infoview_methods() {
        assert_eq!(
            InfoviewRequest::from_method("$/lean/rpc/call"),
            Some(InfoviewRequest::RpcCall)
        );
        assert_eq!(
            InfoviewRequest::from_method("$/lean/plainTermGoal"),
            Some(InfoviewRequest::PlainTermGoal)
        );
        assert_eq!(InfoviewRequest::from_method("textDocument/hover"), None);
        assert_eq!(
            InfoviewNotification::from_method("$/lean/rpc/release"),
            Some(InfoviewNotification::RpcRelease)
        );
        assert_eq!(InfoviewNotification::from_method("$/lean/rpc/call"), None);
    }

    #[test]
    fn reads_session_ids_as_strings() {
        let connected: RpcConnected =
            serde_json::from_str(r#"{"sessionId":"12040831355305299715"}"#).unwrap();
        assert_eq!(
            connected.session_id,
            RpcSessionId("12040831355305299715".into())
        );
    }
}
