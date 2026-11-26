//! Messages between Zed and the infoview page (`assets/bridge.js`).
//!
//! The page posts [`PageMessage`]s as JSON strings. Zed answers by evaluating the
//! scripts that [`InfoviewCall::script`] and [`reply_script`] build.

use project::lsp_store::lean_ext::{NotificationDirection, RpcSessionId};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, value::RawValue};

use crate::context_menu::ContextMenuEntry;

/// Numbers a call from the page, so that its reply and cancellation can name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct CallId(pub u64);

#[derive(Debug, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PageMessage {
    /// The infoview has rendered and takes [`InfoviewCall`]s from now on.
    Ready,
    Call {
        id: CallId,
        call: EditorCall,
    },
    /// The infoview aborted a `sendClientRequest` call.
    Cancel {
        id: CallId,
    },
    Log {
        level: LogLevel,
        message: String,
    },
    /// The user asked for a context menu, at a position in the page.
    ContextMenu {
        /// The `data-vscode-context` of the clicked element and its ancestors, the
        /// closest element's value winning for each key.
        context: Map<String, Value>,
        selection: String,
        x: f32,
        y: f32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    Error,
    Warn,
}

/// A call the infoview makes through its `EditorApi`.
#[derive(Debug, PartialEq, Deserialize)]
#[serde(
    tag = "method",
    content = "params",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum EditorCall {
    SaveConfig {
        config: Value,
    },
    SendClientRequest {
        uri: lsp::Uri,
        method: String,
        params: Value,
    },
    SendClientNotification {
        uri: lsp::Uri,
        method: String,
        params: Value,
    },
    Subscribe {
        direction: NotificationDirection,
        method: String,
    },
    Unsubscribe {
        direction: NotificationDirection,
        method: String,
    },
    CopyToClipboard {
        text: String,
    },
    InsertText {
        text: String,
        kind: TextInsertKind,
        pos: Option<lsp::TextDocumentPositionParams>,
    },
    ApplyEdit {
        edit: lsp::WorkspaceEdit,
    },
    ShowDocument {
        show: lsp::ShowDocumentParams,
    },
    RestartFile {
        uri: lsp::Uri,
    },
    CreateRpcSession {
        uri: lsp::Uri,
    },
    CloseRpcSession {
        session_id: RpcSessionId,
    },
    ReadState {
        container: StateContainer,
        key: String,
    },
    SaveState {
        container: StateContainer,
        key: String,
        value: Value,
    },
    DeleteState {
        container: StateContainer,
        key: String,
    },
}

/// Where `insertText` puts its text: at the position, or on a new line above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextInsertKind {
    Here,
    Above,
}

/// `infoview` entries live as long as the page, `workspace` entries as long as the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StateContainer {
    Infoview,
    Workspace,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ServerStoppedReason {
    pub message: String,
    pub reason: String,
}

/// An infoview command that the editor triggers, such as from a key binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InfoviewActionKind {
    ToggleAllMessages,
    TogglePaused,
    TogglePin,
    CopyToComment,
    ToggleExpectedType,
}

/// A call into the infoview's `InfoviewApi`.
#[derive(Debug)]
pub enum InfoviewCall {
    Initialize(lsp::Location),
    ChangedCursorLocation(Option<lsp::Location>),
    GotServerNotification {
        method: String,
        params: Box<RawValue>,
    },
    SentClientNotification {
        method: String,
        params: Box<RawValue>,
    },
    /// Carries the server's `InitializeResult`.
    ServerRestarted(Value),
    ServerStopped(Option<ServerStoppedReason>),
    ChangedInfoviewConfig(Value),
    RequestedAction(InfoviewActionKind),
    ClickedContextMenu {
        entry: ContextMenuEntry,
        id: Value,
    },
}

impl InfoviewCall {
    /// Returns JavaScript that makes this call in the page.
    pub fn script(&self) -> serde_json::Result<String> {
        let (method, args) = match self {
            Self::Initialize(location) => ("initialize", serde_json::to_string(&(location,))?),
            Self::ChangedCursorLocation(location) => (
                "changedCursorLocation",
                serde_json::to_string(&(location,))?,
            ),
            Self::GotServerNotification { method, params } => (
                "gotServerNotification",
                serde_json::to_string(&(method, params))?,
            ),
            Self::SentClientNotification { method, params } => (
                "sentClientNotification",
                serde_json::to_string(&(method, params))?,
            ),
            Self::ServerRestarted(result) => {
                ("serverRestarted", serde_json::to_string(&(result,))?)
            }
            Self::ServerStopped(reason) => ("serverStopped", serde_json::to_string(&(reason,))?),
            Self::ChangedInfoviewConfig(config) => {
                ("changedInfoviewConfig", serde_json::to_string(&(config,))?)
            }
            Self::RequestedAction(kind) => (
                "requestedAction",
                serde_json::to_string(&(serde_json::json!({ "kind": kind }),))?,
            ),
            Self::ClickedContextMenu { entry, id } => (
                "clickedContextMenu",
                serde_json::to_string(&(serde_json::json!({ "entry": entry, "id": id }),))?,
            ),
        };
        Ok(format!("window.zedInfoview.call(\"{method}\",{args})"))
    }
}

/// The error a page call rejects with, shaped like an LSP `ResponseError` so that the
/// infoview can read RPC error codes such as `RpcNeedsReconnect`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CallError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl CallError {
    const INTERNAL_ERROR: i64 = -32603;

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: Self::INTERNAL_ERROR,
            message: message.into(),
            data: None,
        }
    }
}

impl From<anyhow::Error> for CallError {
    fn from(error: anyhow::Error) -> Self {
        match error.downcast_ref::<lsp::ResponseError>() {
            Some(response) => Self {
                code: response.code.into(),
                message: response.message.clone(),
                data: response.data.clone(),
            },
            None => Self::internal(format!("{error:#}")),
        }
    }
}

pub type CallResult = Result<Value, CallError>;

/// Returns JavaScript that settles the page's pending call `id`.
pub fn reply_script(id: CallId, result: &CallResult) -> serde_json::Result<String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    enum Outcome<'a> {
        Ok(&'a Value),
        Error(&'a CallError),
    }

    let outcome = match result {
        Ok(value) => Outcome::Ok(value),
        Err(error) => Outcome::Error(error),
    };
    Ok(format!(
        "window.zedInfoview.reply({},{})",
        id.0,
        serde_json::to_string(&outcome)?
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageAppearance {
    Light,
    Dark,
}

/// Returns JavaScript that reloads `theme.css` and switches the page's light or dark styles.
pub fn appearance_script(appearance: PageAppearance) -> String {
    let appearance = match appearance {
        PageAppearance::Light => "light",
        PageAppearance::Dark => "dark",
    };
    format!("window.zedInfoview.setAppearance(\"{appearance}\")")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(message: &str) -> PageMessage {
        serde_json::from_str(message).unwrap()
    }

    #[test]
    fn parses_page_messages() {
        assert_eq!(parse(r#"{"kind":"ready"}"#), PageMessage::Ready);
        assert_eq!(
            parse(r#"{"kind":"cancel","id":4}"#),
            PageMessage::Cancel { id: CallId(4) }
        );
        assert_eq!(
            parse(r#"{"kind":"log","level":"warn","message":"slow"}"#),
            PageMessage::Log {
                level: LogLevel::Warn,
                message: "slow".into()
            }
        );
        assert_eq!(
            parse(
                r#"{"kind":"contextMenu","context":{"pinId":"2"},"selection":"","x":10,"y":20.5}"#
            ),
            PageMessage::ContextMenu {
                context: json!({"pinId": "2"}).as_object().unwrap().clone(),
                selection: String::new(),
                x: 10.,
                y: 20.5,
            }
        );
    }

    #[test]
    fn parses_editor_calls() {
        assert_eq!(
            parse(
                r#"{"kind":"call","id":7,"call":{"method":"sendClientRequest","params":{"uri":"file:///p/A.lean","method":"$/lean/rpc/call","params":{"sessionId":"12"}}}}"#
            ),
            PageMessage::Call {
                id: CallId(7),
                call: EditorCall::SendClientRequest {
                    uri: "file:///p/A.lean".parse().unwrap(),
                    method: "$/lean/rpc/call".into(),
                    params: json!({"sessionId": "12"}),
                },
            }
        );
        assert_eq!(
            parse(
                r#"{"kind":"call","id":8,"call":{"method":"subscribe","params":{"direction":"fromServer","method":"$/lean/fileProgress"}}}"#
            ),
            PageMessage::Call {
                id: CallId(8),
                call: EditorCall::Subscribe {
                    direction: NotificationDirection::FromServer,
                    method: "$/lean/fileProgress".into(),
                },
            }
        );
        assert_eq!(
            parse(
                r#"{"kind":"call","id":9,"call":{"method":"closeRpcSession","params":{"sessionId":"77"}}}"#
            ),
            PageMessage::Call {
                id: CallId(9),
                call: EditorCall::CloseRpcSession {
                    session_id: RpcSessionId("77".into()),
                },
            }
        );
        assert_eq!(
            parse(
                r#"{"kind":"call","id":10,"call":{"method":"insertText","params":{"text":"simp","kind":"above"}}}"#
            ),
            PageMessage::Call {
                id: CallId(10),
                call: EditorCall::InsertText {
                    text: "simp".into(),
                    kind: TextInsertKind::Above,
                    pos: None,
                },
            }
        );
        assert_eq!(
            parse(
                r#"{"kind":"call","id":11,"call":{"method":"readState","params":{"container":"workspace","key":"k"}}}"#
            ),
            PageMessage::Call {
                id: CallId(11),
                call: EditorCall::ReadState {
                    container: StateContainer::Workspace,
                    key: "k".into(),
                },
            }
        );
    }

    #[test]
    fn rejects_unknown_editor_calls() {
        let message = r#"{"kind":"call","id":1,"call":{"method":"launchMissiles","params":{}}}"#;
        assert!(serde_json::from_str::<PageMessage>(message).is_err());
    }

    #[test]
    fn builds_infoview_call_scripts() {
        let params =
            RawValue::from_string(r#"{"uri":"file:///p/A.lean","diagnostics":[]}"#.into()).unwrap();
        assert_eq!(
            InfoviewCall::GotServerNotification {
                method: "textDocument/publishDiagnostics".into(),
                params,
            }
            .script()
            .unwrap(),
            r#"window.zedInfoview.call("gotServerNotification",["textDocument/publishDiagnostics",{"uri":"file:///p/A.lean","diagnostics":[]}])"#
        );
        assert_eq!(
            InfoviewCall::ChangedCursorLocation(Some(lsp::Location {
                uri: "file:///p/A.lean".parse().unwrap(),
                range: lsp::Range::new(lsp::Position::new(1, 2), lsp::Position::new(1, 2)),
            }))
            .script()
            .unwrap(),
            r#"window.zedInfoview.call("changedCursorLocation",[{"uri":"file:///p/A.lean","range":{"start":{"line":1,"character":2},"end":{"line":1,"character":2}}}])"#
        );
        assert_eq!(
            InfoviewCall::ServerStopped(None).script().unwrap(),
            r#"window.zedInfoview.call("serverStopped",[null])"#
        );
        assert_eq!(
            InfoviewCall::RequestedAction(InfoviewActionKind::ToggleAllMessages)
                .script()
                .unwrap(),
            r#"window.zedInfoview.call("requestedAction",[{"kind":"toggleAllMessages"}])"#
        );
        assert_eq!(
            InfoviewCall::ClickedContextMenu {
                entry: ContextMenuEntry::CopyState,
                id: json!("goals-3"),
            }
            .script()
            .unwrap(),
            r#"window.zedInfoview.call("clickedContextMenu",[{"entry":"copyState","id":"goals-3"}])"#
        );
    }

    #[test]
    fn builds_reply_scripts() {
        assert_eq!(
            reply_script(CallId(3), &Ok(json!({"sessionId": "5"}))).unwrap(),
            r#"window.zedInfoview.reply(3,{"ok":{"sessionId":"5"}})"#
        );
        let error = CallError::from(anyhow::Error::new(lsp::ResponseError::new(
            lsp::ResponseErrorCode::Other(-32900),
            "Outdated RPC session",
        )));
        assert_eq!(
            reply_script(CallId(4), &Err(error)).unwrap(),
            r#"window.zedInfoview.reply(4,{"error":{"code":-32900,"message":"Outdated RPC session"}})"#
        );
        let error = CallError::from(anyhow::anyhow!("no Lean server"));
        assert_eq!(
            reply_script(CallId(5), &Err(error)).unwrap(),
            r#"window.zedInfoview.reply(5,{"error":{"code":-32603,"message":"no Lean server"}})"#
        );
    }
}
