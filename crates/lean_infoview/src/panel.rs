use std::{
    cell::{Cell, RefCell},
    ops::Range,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context as _, Result, anyhow};
use collections::HashMap;
use db::kvp::KeyValueStore;
use editor::{Editor, EditorEvent};
use futures::{StreamExt as _, channel::mpsc};
use gpui::{
    Action, App, AsyncWindowContext, Bounds, ClipboardItem, Context, DismissEvent, Entity,
    EventEmitter, FocusHandle, Focusable, Pixels, Render, Subscription, Task, WeakEntity, Window,
    anchored, canvas, deferred, div, point, prelude::*, px,
};
use language::{Buffer, LocalFile as _, Point, PointUtf16, ToPointUtf16 as _, Unclipped};
use lsp::{LanguageServer, LanguageServerId};
use project::{
    Project,
    lsp_command::file_path_to_lsp_url,
    lsp_store::{
        LspStoreEvent,
        lean_ext::{
            InfoviewNotification, InfoviewRequest, LEAN_SERVER_NAME, ObservedNotification,
            RpcConnect, RpcConnectParams, RpcKeepAlive, observe_notifications,
        },
    },
    project_settings::ProjectSettings,
};
use serde_json::{Value, json};
use settings::{Settings as _, SettingsStore};
use theme::ActiveTheme as _;
use theme_settings::ThemeSettings;
use ui::{ContextMenu, IconName, Label, LabelCommon as _, LabelSize};
use workspace::{
    OpenOptions, Workspace,
    dock::{DockPosition, Panel, PanelEvent},
};

use crate::{
    CopyToComment, ToggleAllMessages, ToggleExpectedType, ToggleInfoview, TogglePaused, TogglePin,
    assets,
    context_menu::{MenuGroup, OfferedEntry, offered_entries},
    host::{HostState, RpcSession},
    protocol::{
        CallError, CallId, CallResult, EditorCall, InfoviewActionKind, InfoviewCall,
        InfoviewConfig, LogLevel, PageAppearance, PageMessage, ServerStoppedReason, StateContainer,
        TextInsertKind, appearance_script, reply_script,
    },
    settings::LeanInfoviewSettings,
    theme_css::{self, PageFonts},
    web_view::{Resource, WebView, WebViewEvent},
};

const LEAN_LANGUAGE_NAME: &str = "Lean 4";
const KEEP_ALIVE_PERIOD: Duration = Duration::from_secs(10);
const REQUEST_CANCELLED: i64 = -32800;

pub fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, _, _| {
        workspace.register_action(|workspace, _: &ToggleInfoview, window, cx| {
            let Some(panel) = workspace.panel::<LeanInfoviewPanel>(cx) else {
                return;
            };
            let visible = workspace.all_docks().iter().any(|dock| {
                dock.read(cx)
                    .visible_panel()
                    .is_some_and(|visible| visible.panel_id() == panel.entity_id())
            });
            if visible {
                workspace.close_panel::<LeanInfoviewPanel>(window, cx);
            } else {
                workspace.open_panel::<LeanInfoviewPanel>(window, cx);
            }
        });
        register_infoview_action::<ToggleAllMessages>(
            workspace,
            InfoviewActionKind::ToggleAllMessages,
        );
        register_infoview_action::<TogglePaused>(workspace, InfoviewActionKind::TogglePaused);
        register_infoview_action::<TogglePin>(workspace, InfoviewActionKind::TogglePin);
        register_infoview_action::<CopyToComment>(workspace, InfoviewActionKind::CopyToComment);
        register_infoview_action::<ToggleExpectedType>(
            workspace,
            InfoviewActionKind::ToggleExpectedType,
        );
    })
    .detach();
}

fn register_infoview_action<A: Action>(workspace: &mut Workspace, kind: InfoviewActionKind) {
    workspace.register_action(move |workspace, _: &A, window, cx| {
        let Some(panel) = workspace.panel::<LeanInfoviewPanel>(cx) else {
            return;
        };
        // Like vscode-lean4's "Toggle All Messages", which shows the infoview first.
        if kind == InfoviewActionKind::ToggleAllMessages {
            workspace.open_panel::<LeanInfoviewPanel>(window, cx);
        }
        panel.update(cx, |panel, _| panel.request_action(kind));
    });
}

pub struct LeanInfoviewPanel {
    workspace: WeakEntity<Workspace>,
    project: Entity<Project>,
    focus_handle: FocusHandle,
    position: DockPosition,
    active: bool,
    /// Like VS Code, the panel opens by itself for the first Lean file, once, unless the
    /// `auto_open` setting says otherwise.
    auto_opened: bool,
    page: Option<Page>,
    host: HostState,
    servers: HashMap<LanguageServerId, ServerTap>,
    cursor: Option<CursorSource>,
    /// The config last sent to the page.
    config: Option<InfoviewConfig>,
    /// The key-value store key of the infoview's `workspace` state, once read from it.
    workspace_state_key: Option<String>,
    state_writes: mpsc::UnboundedSender<(String, String)>,
    theme_css: Rc<RefCell<String>>,
    /// Where the page is in the window, to place its context menu.
    page_bounds: Rc<Cell<Bounds<Pixels>>>,
    context_menu: Option<DeployedContextMenu>,
    notifications: mpsc::UnboundedSender<(LanguageServerId, ObservedNotification)>,
    _tasks: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

struct DeployedContextMenu {
    menu: Entity<ContextMenu>,
    position: gpui::Point<Pixels>,
    _dismiss: Subscription,
}

struct ServerTap {
    server: Arc<LanguageServer>,
    _notifications: lsp::Subscription,
}

/// The Lean editor whose cursor the infoview follows.
struct CursorSource {
    editor: WeakEntity<Editor>,
    _events: Subscription,
}

/// Where the cursor is, in a document that a Lean server has open.
struct Cursor {
    location: lsp::Location,
    server: LanguageServerId,
}

struct Page {
    web_view: WebView,
    state: PageState,
    /// Calls the page is waiting on, dropped to cancel them.
    calls: HashMap<CallId, Task<()>>,
    /// Actions requested before the page was initialized.
    requested: Vec<InfoviewActionKind>,
    _messages: Task<()>,
}

enum PageState {
    Loading,
    Ready {
        /// `InfoviewApi.initialize` must be called exactly once, with a cursor location.
        initialized: bool,
        /// The server the page last got `serverRestarted` for.
        announced: Option<LanguageServerId>,
    },
}

impl Page {
    fn evaluate(&self, script: serde_json::Result<String>) {
        match script {
            Ok(script) => self.web_view.evaluate(&script),
            Err(error) => log::error!("failed to encode a Lean infoview call: {error}"),
        }
    }

    fn reply(&self, id: CallId, result: &CallResult) {
        self.evaluate(reply_script(id, result));
    }
}

impl LeanInfoviewPanel {
    pub async fn load(
        workspace: WeakEntity<Workspace>,
        mut cx: AsyncWindowContext,
    ) -> Result<Entity<Self>> {
        workspace.update_in(&mut cx, |workspace, window, cx| {
            let workspace_entity = cx.entity();
            cx.new(|cx| Self::new(workspace, workspace_entity, window, cx))
        })
    }

    fn new(
        workspace: &Workspace,
        workspace_entity: Entity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let project = workspace.project().clone();
        let lsp_store = project.read(cx).lsp_store();
        let (notifications, mut received) = mpsc::unbounded();

        let forward_notifications = cx.spawn(async move |this, cx| {
            while let Some((server, notification)) = received.next().await {
                let observed = this.update(cx, |panel, _| {
                    if let Some(call) = panel.host.observe(server, notification) {
                        panel.send(call);
                    }
                });
                if observed.is_err() {
                    break;
                }
            }
        });
        let (state_writes, mut pending_writes) = mpsc::unbounded::<(String, String)>();
        let db = KeyValueStore::global(cx);
        let write_state = cx.background_spawn(async move {
            while let Some((key, value)) = pending_writes.next().await {
                if let Err(error) = db.write_kvp(key, value).await {
                    log::error!("failed to store the Lean infoview's workspace state: {error:#}");
                }
            }
        });
        let keep_alive = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(KEEP_ALIVE_PERIOD).await;
                if this.update(cx, |panel, _| panel.keep_alive()).is_err() {
                    break;
                }
            }
        });

        let subscriptions = vec![
            cx.subscribe(&lsp_store, |panel, lsp_store, event, cx| match event {
                LspStoreEvent::LanguageServerAdded(id, name, _) if *name == LEAN_SERVER_NAME => {
                    if let Some(server) = lsp_store.read(cx).language_server_for_id(*id) {
                        panel.tap_server(server);
                        panel.sync_page(cx);
                    }
                }
                LspStoreEvent::LanguageServerRemoved(id) => panel.server_removed(*id, cx),
                _ => {}
            }),
            cx.subscribe_in(
                &workspace_entity,
                window,
                |panel, workspace, event, window, cx| {
                    if let workspace::Event::ActiveItemChanged = event {
                        let editor = active_editor(workspace.read(cx), cx);
                        panel.follow_editor(editor, window, cx);
                    }
                },
            ),
            cx.observe_global_in::<SettingsStore>(window, |panel, _, cx| {
                panel.update_theme(cx);
                panel.update_config(cx);
            }),
        ];

        let mut panel = Self {
            workspace: workspace.weak_handle(),
            project,
            focus_handle: cx.focus_handle(),
            position: DockPosition::Right,
            active: false,
            auto_opened: false,
            page: None,
            host: HostState::default(),
            servers: HashMap::default(),
            cursor: None,
            config: None,
            workspace_state_key: None,
            state_writes,
            theme_css: Rc::default(),
            page_bounds: Rc::default(),
            context_menu: None,
            notifications,
            _tasks: vec![forward_notifications, keep_alive, write_state],
            _subscriptions: subscriptions,
        };
        let running: Vec<_> = lsp_store
            .read(cx)
            .language_server_statuses()
            .filter(|(_, status)| status.name == LEAN_SERVER_NAME)
            .filter_map(|(id, _)| lsp_store.read(cx).language_server_for_id(id))
            .collect();
        for server in running {
            panel.tap_server(server);
        }
        panel.update_theme(cx);
        cx.spawn_in(window, async move |this, cx| {
            let editor =
                workspace_entity.read_with(cx, |workspace, cx| active_editor(workspace, cx));
            this.update_in(cx, |panel, window, cx| {
                panel.follow_editor(editor, window, cx)
            })
            .ok();
        })
        .detach();
        panel
    }

    fn tap_server(&mut self, server: Arc<LanguageServer>) {
        let id = server.server_id();
        let notifications = self.notifications.clone();
        let subscription = observe_notifications(&server, move |notification| {
            notifications.unbounded_send((id, notification)).ok();
        });
        self.servers.insert(
            id,
            ServerTap {
                server,
                _notifications: subscription,
            },
        );
    }

    fn server_removed(&mut self, id: LanguageServerId, cx: &mut Context<Self>) {
        if self.servers.remove(&id).is_none() {
            return;
        }
        self.host.server_removed(id);
        if let Some(page) = &mut self.page
            && let PageState::Ready { announced, .. } = &mut page.state
            && *announced == Some(id)
        {
            *announced = None;
            page.evaluate(
                InfoviewCall::ServerStopped(Some(ServerStoppedReason {
                    message: "The Lean server stopped.".into(),
                    reason: String::new(),
                }))
                .script(),
            );
        }
        cx.notify();
    }

    fn keep_alive(&self) {
        for (server, params) in self.host.keep_alives() {
            if let Some(tap) = self.servers.get(&server) {
                tap.server.notify::<RpcKeepAlive>(params).ok();
            }
        }
    }

    fn follow_editor(
        &mut self,
        editor: Option<Entity<Editor>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = editor else {
            return;
        };
        if lean_selection(editor.read(cx), cx).is_none() {
            return;
        }
        if self
            .cursor
            .as_ref()
            .is_some_and(|cursor| cursor.editor == editor.downgrade())
        {
            return;
        }
        let events = cx.subscribe_in(&editor, window, |panel, _, event, _, cx| {
            if matches!(
                event,
                EditorEvent::SelectionsChanged { .. } | EditorEvent::BufferEdited
            ) {
                panel.sync_page(cx);
            }
        });
        self.cursor = Some(CursorSource {
            editor: editor.downgrade(),
            _events: events,
        });
        if !self.auto_opened && LeanInfoviewSettings::get_global(cx).auto_open {
            let workspace = self.workspace.clone();
            window.defer(cx, move |window, cx| {
                workspace
                    .update(cx, |workspace, cx| {
                        workspace.open_panel::<Self>(window, cx);
                    })
                    .ok();
            });
        }
        self.auto_opened = true;
        self.sync_page(cx);
    }

    fn cursor(&self, cx: &App) -> Option<Cursor> {
        let editor = self.cursor.as_ref()?.editor.upgrade()?;
        let (buffer, range) = lean_selection(editor.read(cx), cx)?;
        let location = location(&buffer, range, cx)?;
        let server = self.project.read(cx).language_server_id_for_name(
            buffer.read(cx),
            &LEAN_SERVER_NAME,
            cx,
        )?;
        Some(Cursor { location, server })
    }

    /// Tells a ready page about the cursor, and about the server when it changed.
    fn sync_page(&mut self, cx: &App) {
        let cursor = self.cursor(cx);
        let Some(page) = &mut self.page else {
            return;
        };
        let PageState::Ready {
            initialized,
            announced,
        } = &mut page.state
        else {
            return;
        };
        let Some(cursor) = cursor else {
            return;
        };
        let mut calls = Vec::new();
        if !*initialized {
            *initialized = true;
            calls.push(InfoviewCall::Initialize(cursor.location.clone()));
        }
        if *announced != Some(cursor.server)
            && let Some(tap) = self.servers.get(&cursor.server)
        {
            *announced = Some(cursor.server);
            calls.push(InfoviewCall::ServerStopped(None));
            calls.push(InfoviewCall::ServerRestarted(initialize_result(
                &tap.server,
            )));
        }
        calls.push(InfoviewCall::ChangedCursorLocation(Some(cursor.location)));
        calls.extend(page.requested.drain(..).map(InfoviewCall::RequestedAction));
        for call in calls {
            page.evaluate(call.script());
        }
    }

    fn request_action(&mut self, kind: InfoviewActionKind) {
        let Some(page) = &mut self.page else {
            return;
        };
        match page.state {
            PageState::Ready {
                initialized: true, ..
            } => page.evaluate(InfoviewCall::RequestedAction(kind).script()),
            _ => page.requested.push(kind),
        }
    }

    /// Sends a call to the page once it is ready. Earlier calls are covered by what
    /// the page gets when it becomes ready.
    fn send(&self, call: InfoviewCall) {
        if let Some(page) = &self.page
            && let PageState::Ready { .. } = page.state
        {
            page.evaluate(call.script());
        }
    }

    fn update_theme(&mut self, cx: &mut Context<Self>) {
        let settings = ThemeSettings::get_global(cx);
        let buffer_font_size = settings.buffer_font_size(cx).as_f32();
        let fonts = PageFonts {
            ui_family: settings.ui_font.family.to_string(),
            ui_size: settings.ui_font_size(cx).as_f32(),
            ui_weight: settings.ui_font.weight.0,
            buffer_family: settings.buffer_font.family.to_string(),
            buffer_size: buffer_font_size,
            buffer_weight: settings.buffer_font.weight.0,
            buffer_line_height: (settings.line_height() * buffer_font_size).round(),
        };
        let theme = cx.theme();
        let css = theme_css::stylesheet(theme.colors(), theme.status(), &fonts);
        if *self.theme_css.borrow() == css {
            return;
        }
        *self.theme_css.borrow_mut() = css;
        if let Some(page) = &self.page {
            page.web_view
                .native_view()
                .set_background(theme.colors().panel_background, cx);
            page.web_view
                .evaluate(&appearance_script(page_appearance(cx)));
        }
    }

    fn open_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (events, mut received) = mpsc::unbounded();
        let theme_css = self.theme_css.clone();
        let fonts = cx.asset_source().clone();
        let resolve = Box::new(move |path: &str| match path {
            "/theme.css" => Some(Resource {
                mime_type: "text/css; charset=utf-8".into(),
                bytes: theme_css.borrow().clone().into_bytes().into(),
            }),
            _ => match path
                .strip_prefix('/')
                .filter(|path| path.starts_with("fonts/"))
            {
                Some(font) => Some(Resource {
                    mime_type: "font/ttf".into(),
                    bytes: fonts.load(font).ok()??,
                }),
                None => assets::asset(path).map(|asset| Resource {
                    mime_type: asset.mime_type.into(),
                    bytes: asset.bytes.into(),
                }),
            },
        });
        let web_view = match WebView::new(resolve, events, window, cx) {
            Ok(web_view) => web_view,
            Err(error) => {
                log::error!("failed to create the Lean infoview: {error:#}");
                return;
            }
        };
        let native_view = web_view.native_view();
        native_view.reserve_action::<ToggleInfoview>();
        native_view.reserve_action::<ToggleAllMessages>();
        native_view.reserve_action::<TogglePaused>();
        native_view.reserve_action::<TogglePin>();
        native_view.reserve_action::<CopyToComment>();
        native_view.reserve_action::<ToggleExpectedType>();
        web_view
            .native_view()
            .set_background(cx.theme().colors().panel_background, cx);
        let receive_messages = cx.spawn_in(window, async move |this, cx| {
            while let Some(event) = received.next().await {
                let handled = this.update_in(cx, |panel, window, cx| match event {
                    WebViewEvent::Message(message) => {
                        panel.handle_page_message(&message, window, cx)
                    }
                    WebViewEvent::Link(url) => cx.open_url(&url),
                });
                if handled.is_err() {
                    break;
                }
            }
        });
        self.page = Some(Page {
            web_view,
            state: PageState::Loading,
            calls: HashMap::default(),
            requested: Vec::new(),
            _messages: receive_messages,
        });
        cx.notify();
    }

    fn handle_page_message(&mut self, message: &str, window: &mut Window, cx: &mut Context<Self>) {
        let message = match serde_json::from_str::<PageMessage>(message) {
            Ok(message) => message,
            Err(error) => {
                log::error!("unexpected message from the Lean infoview: {error}: {message}");
                return;
            }
        };
        match message {
            PageMessage::Ready => self.page_ready(cx),
            PageMessage::Call { id, call } => self.handle_call(id, call, window, cx),
            PageMessage::Cancel { id } => {
                if let Some(page) = &mut self.page
                    && page.calls.remove(&id).is_some()
                {
                    page.reply(
                        id,
                        &Err(CallError {
                            code: REQUEST_CANCELLED,
                            message: "request cancelled".into(),
                            data: None,
                        }),
                    );
                }
            }
            PageMessage::Log { level, message } => match level {
                LogLevel::Error => log::error!("Lean infoview: {message}"),
                LogLevel::Warn => log::warn!("Lean infoview: {message}"),
            },
            PageMessage::ContextMenu {
                context,
                selection,
                x,
                y,
            } => {
                let position = self.page_bounds.get().origin + point(px(x), px(y));
                self.deploy_context_menu(
                    offered_entries(&context),
                    selection,
                    position,
                    window,
                    cx,
                );
            }
        }
    }

    fn deploy_context_menu(
        &mut self,
        entries: Vec<OfferedEntry>,
        selection: String,
        position: gpui::Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if entries.is_empty() && selection.is_empty() {
            self.context_menu = None;
            cx.notify();
            return;
        }
        let this = cx.weak_entity();
        let menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
            if !selection.is_empty() {
                menu = menu.entry("Copy", None, move |_, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(selection.clone()))
                });
            }
            let (settings, main): (Vec<_>, Vec<_>) = entries.into_iter().partition(|offered| {
                matches!(offered.group, MenuGroup::Settings | MenuGroup::SaveSettings)
            });
            menu = add_entries(menu, &this, &main);
            if !settings.is_empty() {
                menu = menu.separator().submenu("Settings…", move |menu, _, _| {
                    add_entries(menu, &this, &settings)
                });
            }
            menu
        });
        window.focus(&menu.focus_handle(cx), cx);
        let dismiss = cx.subscribe(&menu, |panel, _, _: &DismissEvent, cx| {
            panel.context_menu = None;
            cx.notify();
        });
        self.context_menu = Some(DeployedContextMenu {
            menu,
            position,
            _dismiss: dismiss,
        });
        cx.notify();
    }

    fn page_ready(&mut self, cx: &mut Context<Self>) {
        let Some(page) = &mut self.page else {
            return;
        };
        self.host.page_loaded();
        page.calls.clear();
        page.state = PageState::Ready {
            initialized: false,
            announced: None,
        };
        page.web_view
            .evaluate(&appearance_script(page_appearance(cx)));
        let config = LeanInfoviewSettings::get_global(cx).config.clone();
        page.evaluate(InfoviewCall::ChangedInfoviewConfig(config.clone()).script());
        self.config = Some(config);
        log::info!("Lean infoview ready");
        self.sync_page(cx);
    }

    fn update_config(&mut self, cx: &App) {
        let config = &LeanInfoviewSettings::get_global(cx).config;
        if self.config.as_ref() == Some(config) {
            return;
        }
        self.config = Some(config.clone());
        self.send(InfoviewCall::ChangedInfoviewConfig(config.clone()));
    }

    /// Writes the config to the user's settings, as vscode-lean4 does. The settings
    /// change then reaches the page.
    fn save_config(&self, config: Value, cx: &App) -> Result<()> {
        let config: InfoviewConfig =
            serde_json::from_value(config).context("invalid infoview config")?;
        settings::update_settings_file(
            self.project.read(cx).fs().clone(),
            cx,
            move |settings, _| {
                config.write_to(
                    settings
                        .lean
                        .get_or_insert_default()
                        .infoview
                        .get_or_insert_default(),
                )
            },
        );
        Ok(())
    }

    /// Reads the `workspace` state that earlier sessions stored, on first use.
    fn load_workspace_state(&mut self, container: StateContainer, cx: &App) {
        if container != StateContainer::Workspace || self.workspace_state_key.is_some() {
            return;
        }
        let key = workspace_state_key(&self.project, cx);
        let stored = KeyValueStore::global(cx)
            .read_kvp(&key)
            .context("reading the Lean infoview's workspace state")
            .and_then(|stored| {
                stored
                    .map(|stored| serde_json::from_str(&stored))
                    .transpose()
                    .context("parsing the Lean infoview's workspace state")
            });
        match stored {
            Ok(Some(entries)) => self.host.load_workspace_state(entries),
            Ok(None) => {}
            Err(error) => log::error!("{error:#}"),
        }
        self.workspace_state_key = Some(key);
    }

    fn store_workspace_state(&self, container: StateContainer) {
        if container != StateContainer::Workspace {
            return;
        }
        let Some(key) = self.workspace_state_key.clone() else {
            return;
        };
        match serde_json::to_string(&self.host.workspace_state()) {
            Ok(entries) => {
                self.state_writes.unbounded_send((key, entries)).ok();
            }
            Err(error) => {
                log::error!("failed to encode the Lean infoview's workspace state: {error}")
            }
        }
    }

    fn handle_call(
        &mut self,
        id: CallId,
        call: EditorCall,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = match call {
            EditorCall::SaveConfig { config } => self
                .save_config(config, cx)
                .map(|()| Value::Null)
                .map_err(CallError::from),
            EditorCall::SendClientRequest {
                uri,
                method,
                params,
            } => return self.send_request(id, &uri, &method, params, window, cx),
            EditorCall::SendClientNotification {
                uri,
                method,
                params,
            } => self.send_notification(&uri, &method, params, cx),
            EditorCall::Subscribe { direction, method } => {
                let replay = self.host.subscribe(direction, method);
                self.reply(id, &Ok(Value::Null));
                for call in replay {
                    self.send(call);
                }
                return;
            }
            EditorCall::Unsubscribe { direction, method } => self
                .host
                .unsubscribe(direction, method)
                .map(|()| Value::Null)
                .map_err(CallError::from),
            EditorCall::CopyToClipboard { text } => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                Ok(Value::Null)
            }
            EditorCall::InsertText { text, kind, pos } => self
                .insert_text(text, kind, pos, window, cx)
                .map(|()| Value::Null)
                .map_err(CallError::from),
            EditorCall::ApplyEdit { edit } => {
                let project = self.project.clone();
                return self.spawn_call(id, window, cx, async move |_, cx| {
                    apply_workspace_edit(project, edit, cx).await?;
                    Ok(Value::Null)
                });
            }
            EditorCall::ShowDocument { show } => {
                let workspace = self.workspace.clone();
                return self.spawn_call(id, window, cx, async move |_, cx| {
                    show_document(workspace, show, cx).await?;
                    Ok(Value::Null)
                });
            }
            EditorCall::RestartFile { uri } => self
                .restart_file(&uri, cx)
                .map(|()| Value::Null)
                .map_err(CallError::from),
            EditorCall::CreateRpcSession { uri } => {
                return self.create_rpc_session(id, uri, window, cx);
            }
            EditorCall::CloseRpcSession { session_id } => {
                self.host.close_rpc_session(&session_id);
                Ok(Value::Null)
            }
            EditorCall::ReadState { container, key } => {
                self.load_workspace_state(container, cx);
                Ok(self.host.read_state(container, key))
            }
            EditorCall::SaveState {
                container,
                key,
                value,
            } => {
                self.load_workspace_state(container, cx);
                self.host.save_state(container, key, value);
                self.store_workspace_state(container);
                Ok(Value::Null)
            }
            EditorCall::DeleteState { container, key } => {
                self.load_workspace_state(container, cx);
                self.host.delete_state(container, key);
                self.store_workspace_state(container);
                Ok(Value::Null)
            }
        };
        self.reply(id, &result);
    }

    fn reply(&self, id: CallId, result: &CallResult) {
        if let Some(page) = &self.page {
            page.reply(id, result);
        }
    }

    /// Runs a call that finishes later, and replies when it does.
    fn spawn_call(
        &mut self,
        id: CallId,
        window: &mut Window,
        cx: &mut Context<Self>,
        call: impl AsyncFnOnce(WeakEntity<Self>, &mut AsyncWindowContext) -> Result<Value> + 'static,
    ) {
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = call(this.clone(), cx).await.map_err(CallError::from);
            this.update(cx, |panel, _| {
                if let Some(page) = &mut panel.page
                    && page.calls.remove(&id).is_some()
                {
                    page.reply(id, &result);
                }
            })
            .ok();
        });
        if let Some(page) = &mut self.page {
            page.calls.insert(id, task);
        }
    }

    fn send_request(
        &mut self,
        id: CallId,
        uri: &lsp::Uri,
        method: &str,
        params: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let request = InfoviewRequest::from_method(method)
            .ok_or_else(|| anyhow::Error::new(lsp::ResponseError::method_not_found(method)))
            .and_then(|request| Ok((request, self.server_for(uri, cx)?)));
        let (request, server) = match request {
            Ok(request) => request,
            Err(error) => return self.reply(id, &Err(error.into())),
        };
        // The server answers RPC calls only once the file is elaborated up to the
        // position, which has no useful bound. The infoview cancels what it stops needing.
        let response = request.send(&server, params, Duration::MAX);
        self.spawn_call(id, window, cx, async move |_, _| response.await);
    }

    fn send_notification(
        &self,
        uri: &lsp::Uri,
        method: &str,
        params: Value,
        cx: &App,
    ) -> CallResult {
        let notification = InfoviewNotification::from_method(method)
            .ok_or_else(|| anyhow::Error::new(lsp::ResponseError::method_not_found(method)))?;
        let server = self.server_for(uri, cx)?;
        notification.send(&server, params)?;
        Ok(Value::Null)
    }

    fn create_rpc_session(
        &mut self,
        id: CallId,
        uri: lsp::Uri,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let server = match self.server_for(&uri, cx) {
            Ok(server) => server,
            Err(error) => return self.reply(id, &Err(error.into())),
        };
        let timeout = ProjectSettings::get_global(cx)
            .global_lsp_settings
            .get_request_timeout();
        let connect = server.request::<RpcConnect>(RpcConnectParams { uri: uri.clone() }, timeout);
        let server = server.server_id();
        self.spawn_call(id, window, cx, async move |this, cx| {
            let connected = connect.await.into_response()?;
            let session_id = connected.session_id;
            this.update(cx, |panel, _| {
                panel
                    .host
                    .open_rpc_session(session_id.clone(), RpcSession { server, uri });
            })?;
            Ok(json!(session_id.0))
        });
    }

    fn server_for(&self, uri: &lsp::Uri, cx: &App) -> Result<Arc<LanguageServer>> {
        let id = open_buffer(&self.project, uri, cx)
            .and_then(|buffer| {
                self.project.read(cx).language_server_id_for_name(
                    buffer.read(cx),
                    &LEAN_SERVER_NAME,
                    cx,
                )
            })
            .or_else(|| self.cursor(cx).map(|cursor| cursor.server))
            .context("no Lean server has this document open")?;
        self.servers
            .get(&id)
            .map(|tap| tap.server.clone())
            .context("the Lean server is not running")
    }

    fn insert_text(
        &mut self,
        text: String,
        kind: TextInsertKind,
        position: Option<lsp::TextDocumentPositionParams>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let editor = self
            .cursor
            .as_ref()
            .and_then(|cursor| cursor.editor.upgrade())
            .context("no Lean editor to insert into")?;
        editor.update(cx, |editor, cx| -> Result<()> {
            let (buffer, selection) =
                lean_selection(editor, cx).context("the editor shows no Lean file")?;
            let snapshot = buffer.read(cx).snapshot();
            let point = match &position {
                Some(position) => utf16_point(position.position, &snapshot),
                None => text::ToPoint::to_point(&selection.end, &snapshot),
            };
            let (at, text) = match kind {
                TextInsertKind::Here => (point, text),
                TextInsertKind::Above => {
                    let indent = " ".repeat(snapshot.indent_size_for_line(point.row).len as usize);
                    let text = text.replace('\n', &format!("\n{indent}"));
                    (Point::new(point.row, 0), format!("{indent}{text}\n"))
                }
            };
            buffer.update(cx, |buffer, cx| buffer.edit([(at..at, text)], None, cx));
            Ok(())
        })?;
        window.focus(&editor.focus_handle(cx), cx);
        Ok(())
    }

    fn restart_file(&self, uri: &lsp::Uri, cx: &App) -> Result<()> {
        let buffer = open_buffer(&self.project, uri, cx).context("the file is not open")?;
        self.project
            .read(cx)
            .lsp_store()
            .read(cx)
            .restart_lean_file(buffer.read(cx), cx)
    }
}

/// Adds the entries, with a separator wherever the group changes.
fn add_entries(
    mut menu: ContextMenu,
    panel: &WeakEntity<LeanInfoviewPanel>,
    entries: &[OfferedEntry],
) -> ContextMenu {
    let mut group = entries.first().map(|offered| offered.group);
    for offered in entries {
        if group != Some(offered.group) {
            group = Some(offered.group);
            menu = menu.separator();
        }
        let panel = panel.clone();
        let (entry, id) = (offered.entry, offered.id.clone());
        menu = menu.entry(offered.label, None, move |_, cx| {
            panel
                .update(cx, |panel, _| {
                    panel.send(InfoviewCall::ClickedContextMenu {
                        entry,
                        id: id.clone(),
                    })
                })
                .ok();
        });
    }
    menu
}

/// Like VS Code's workspace state, the infoview's `workspace` entries belong to a set of
/// folders.
fn workspace_state_key(project: &Entity<Project>, cx: &App) -> String {
    let mut roots: Vec<String> = project
        .read(cx)
        .visible_worktrees(cx)
        .map(|worktree| worktree.read(cx).abs_path().to_string_lossy().into_owned())
        .collect();
    roots.sort();
    format!(
        "lean_infoview_workspace_state:{}",
        serde_json::Value::from(roots)
    )
}

fn active_editor(workspace: &Workspace, cx: &App) -> Option<Entity<Editor>> {
    workspace
        .active_item(cx)
        .and_then(|item| item.act_as::<Editor>(cx))
}

/// The newest selection of the editor, if it is in a Lean buffer.
fn lean_selection(editor: &Editor, cx: &App) -> Option<(Entity<Buffer>, Range<text::Anchor>)> {
    let multi_buffer = editor.buffer().read(cx);
    let snapshot = multi_buffer.snapshot(cx);
    let selection = editor.selections.newest_anchor();
    let (start, _) = snapshot.anchor_to_buffer_anchor(selection.start)?;
    let end = snapshot
        .anchor_to_buffer_anchor(selection.end)
        .map(|(end, _)| end)
        .filter(|end| end.buffer_id == start.buffer_id)
        .unwrap_or(start);
    let buffer = multi_buffer.buffer(start.buffer_id)?;
    let is_lean = buffer
        .read(cx)
        .language_at(start)
        .is_some_and(|language| language.name() == LEAN_LANGUAGE_NAME);
    is_lean.then_some((buffer, start..end))
}

fn location(
    buffer: &Entity<Buffer>,
    range: Range<text::Anchor>,
    cx: &App,
) -> Option<lsp::Location> {
    let buffer = buffer.read(cx);
    let abs_path = project::File::from_dyn(buffer.file())?.abs_path(cx);
    let snapshot = buffer.snapshot();
    let start = range.start.to_point_utf16(&snapshot);
    let end = range.end.to_point_utf16(&snapshot);
    Some(lsp::Location {
        uri: file_path_to_lsp_url(&abs_path).ok()?,
        range: lsp::Range::new(
            lsp::Position::new(start.row, start.column),
            lsp::Position::new(end.row, end.column),
        ),
    })
}

fn open_buffer(project: &Entity<Project>, uri: &lsp::Uri, cx: &App) -> Option<Entity<Buffer>> {
    let path = uri.to_file_path().ok()?;
    let project = project.read(cx);
    let project_path = project.find_project_path(&path, cx)?;
    project.get_open_buffer(&project_path, cx)
}

fn initialize_result(server: &LanguageServer) -> Value {
    json!({
        "capabilities": server.capabilities(),
        "serverInfo": {
            "name": server.name().0,
            "version": server.version(),
        },
    })
}

fn page_appearance(cx: &App) -> PageAppearance {
    if cx.theme().appearance().is_light() {
        PageAppearance::Light
    } else {
        PageAppearance::Dark
    }
}

fn utf16_point(position: lsp::Position, snapshot: &text::BufferSnapshot) -> Point {
    snapshot.point_utf16_to_point(snapshot.clip_point_utf16(
        Unclipped(PointUtf16::new(position.line, position.character)),
        text::Bias::Left,
    ))
}

async fn apply_workspace_edit(
    project: Entity<Project>,
    edit: lsp::WorkspaceEdit,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    for (uri, edits) in crate::edits::text_edits(edit) {
        let path = uri
            .to_file_path()
            .map_err(|()| anyhow!("not a file URI: {}", uri.as_str()))?;
        let buffer = project
            .update(cx, |project, cx| project.open_local_buffer(path, cx))
            .await?;
        buffer.update(cx, |buffer, cx| {
            let snapshot = buffer.snapshot();
            let edits: Vec<_> = edits
                .into_iter()
                .map(|edit| {
                    let start = utf16_point(edit.range.start, &snapshot);
                    let end = utf16_point(edit.range.end, &snapshot);
                    (start..end, edit.new_text)
                })
                .collect();
            buffer.edit(edits, None, cx);
        });
    }
    Ok(())
}

async fn show_document(
    workspace: WeakEntity<Workspace>,
    show: lsp::ShowDocumentParams,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    let path = show
        .uri
        .to_file_path()
        .map_err(|()| anyhow!("not a file URI: {}", show.uri.as_str()))?;
    let item = workspace
        .update_in(cx, |workspace, window, cx| {
            workspace.open_abs_path(path, OpenOptions::default(), window, cx)
        })?
        .await?;
    if let (Some(selection), Some(editor)) = (show.selection, item.downcast::<Editor>()) {
        editor.update_in(cx, |editor, window, cx| {
            let Some(buffer) = editor.buffer().read(cx).as_singleton() else {
                return;
            };
            let snapshot = buffer.read(cx).text_snapshot();
            let start = utf16_point(selection.start, &snapshot);
            let end = utf16_point(selection.end, &snapshot);
            editor.go_to_singleton_buffer_range(start..end, window, cx);
        })?;
    }
    Ok(())
}

impl Focusable for LeanInfoviewPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<PanelEvent> for LeanInfoviewPanel {}

impl Render for LeanInfoviewPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match &self.page {
            Some(page) => page.web_view.native_view().clone().into_any_element(),
            None => div()
                .p_4()
                .child(
                    Label::new("The Lean InfoView opens with a Lean 4 file.")
                        .size(LabelSize::Small)
                        .color(ui::Color::Muted),
                )
                .into_any_element(),
        };
        let page_bounds = self.page_bounds.clone();
        div()
            .key_context("LeanInfoview")
            .track_focus(&self.focus_handle)
            .relative()
            .size_full()
            .bg(cx.theme().colors().panel_background)
            .child(
                canvas(move |bounds, _, _| page_bounds.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .child(content)
            .children(self.context_menu.as_ref().map(|deployed| {
                deferred(
                    anchored()
                        .position(deployed.position)
                        .snap_to_window_with_margin(px(8.))
                        .child(deployed.menu.clone()),
                )
                .with_priority(1)
            }))
    }
}

impl Panel for LeanInfoviewPanel {
    fn persistent_name() -> &'static str {
        "Lean InfoView"
    }

    fn panel_key() -> &'static str {
        "LeanInfoviewPanel"
    }

    fn position(&self, _: &Window, _: &App) -> DockPosition {
        self.position
    }

    fn position_is_valid(&self, _: DockPosition) -> bool {
        true
    }

    fn set_position(&mut self, position: DockPosition, _: &mut Window, cx: &mut Context<Self>) {
        self.position = position;
        cx.notify();
    }

    fn default_size(&self, _: &Window, _: &App) -> Pixels {
        px(420.)
    }

    fn icon(&self, _: &Window, _: &App) -> Option<IconName> {
        (self.active || !self.servers.is_empty()).then_some(IconName::Book)
    }

    fn icon_tooltip(&self, _: &Window, _: &App) -> Option<&'static str> {
        Some("Lean InfoView")
    }

    fn toggle_action(&self) -> Box<dyn Action> {
        Box::new(ToggleInfoview)
    }

    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.active = active;
        if active && self.page.is_none() {
            self.open_page(window, cx);
        }
    }

    fn activation_priority(&self) -> u32 {
        8
    }
}
