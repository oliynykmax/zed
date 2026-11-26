use std::{sync::Arc, time::Duration};

use anyhow::Context as _;
use futures::{StreamExt as _, channel::mpsc};
use gpui::{
    App, AppContext as _, AsyncWindowContext, Context, Entity, TaskExt, WeakEntity, Window,
};
use language::{Capability, Language, LocalFile as _};
use lsp::LanguageServer;
use multi_buffer::MultiBuffer;
use project::{
    lsp_command::file_path_to_lsp_url,
    lsp_store::lean_ext::{LEAN_SERVER_NAME, PlainGoal},
    project_settings::ProjectSettings,
};
use settings::Settings as _;
use text::ToPointUtf16 as _;
use workspace::SplitDirection;

use crate::{
    Editor, EditorEvent, ShowLeanGoal, element::register_action,
    lsp_ext::find_specific_language_server_in_selection,
};

const REFRESH_DEBOUNCE: Duration = Duration::from_millis(100);

fn is_lean_language(language: &Language) -> bool {
    language.name() == "Lean 4"
}

pub fn apply_related_actions(editor: &Entity<Editor>, window: &mut Window, cx: &mut App) {
    if editor
        .read(cx)
        .buffer()
        .read(cx)
        .all_buffers()
        .into_iter()
        .filter_map(|buffer| buffer.read(cx).language().cloned())
        .any(|language| is_lean_language(&language))
    {
        register_action(editor, window, show_lean_goal);
    }
}

fn show_lean_goal(
    editor: &mut Editor,
    _: &ShowLeanGoal,
    window: &mut Window,
    cx: &mut Context<Editor>,
) {
    let Some(project) = editor.project.clone() else {
        return;
    };
    let Some(workspace) = editor.workspace() else {
        return;
    };
    if project
        .read(cx)
        .lsp_store()
        .read(cx)
        .upstream_client()
        .is_some()
    {
        log::warn!("Lean goal view is not supported in remote projects");
        return;
    }

    let (refresh_tx, mut refresh_rx) = mpsc::unbounded::<()>();
    refresh_tx.unbounded_send(()).ok();
    let subscription = cx.subscribe_self(move |_, event: &EditorEvent, _| {
        if matches!(
            event,
            EditorEvent::SelectionsChanged { .. } | EditorEvent::BufferEdited
        ) {
            refresh_tx.unbounded_send(()).ok();
        }
    });
    let languages = project.read(cx).languages().clone();

    cx.spawn_in(window, async move |source_editor, cx| {
        let _subscription = subscription;
        let markdown = languages.language_for_name("Markdown").await.ok();
        let goal_buffer = project
            .update(cx, |project, cx| project.create_buffer(markdown, false, cx))
            .await?;
        let goal_editor = workspace.update_in(cx, |workspace, window, cx| {
            goal_buffer.update(cx, |buffer, cx| {
                buffer.set_capability(Capability::ReadOnly, cx)
            });
            let multibuffer = cx.new(|cx| {
                MultiBuffer::singleton(goal_buffer.clone(), cx).with_title("Lean Goal".into())
            });
            let goal_editor = cx.new(|cx| {
                let mut editor = Editor::for_multibuffer(multibuffer, None, window, cx);
                editor.set_read_only(true);
                editor
            });
            workspace.split_item(
                SplitDirection::Right,
                Box::new(goal_editor.clone()),
                window,
                cx,
            );
            goal_editor.downgrade()
        })?;

        while refresh_rx.next().await.is_some() {
            cx.background_executor().timer(REFRESH_DEBOUNCE).await;
            while refresh_rx.try_recv().is_ok() {}
            if goal_editor.upgrade().is_none() {
                break;
            }
            let text = match request_plain_goal(&source_editor, cx).await {
                Ok(Some(goal)) if goal.goals.is_empty() => "No goals".to_string(),
                Ok(Some(goal)) => goal.rendered,
                Ok(None) => "No goal at cursor".to_string(),
                Err(error) => format!("Lean goal request failed: {error:#}"),
            };
            goal_buffer.update(cx, |buffer, cx| {
                buffer.set_text(text, cx);
            });
        }
        anyhow::Ok(())
    })
    .detach_and_log_err(cx);
}

async fn request_plain_goal(
    source_editor: &WeakEntity<Editor>,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<Option<project::lsp_store::lean_ext::PlainGoalResult>> {
    let request = source_editor.update(cx, |editor, cx| {
        let (anchor, _, server_id, buffer) = find_specific_language_server_in_selection(
            editor,
            cx,
            is_lean_language,
            LEAN_SERVER_NAME,
        )?;
        let project = editor.project.as_ref()?;
        let server: Arc<LanguageServer> = project
            .read(cx)
            .lsp_store()
            .read(cx)
            .language_server_for_id(server_id)?;
        let buffer = buffer.read(cx);
        let abs_path = project::File::from_dyn(buffer.file())?.abs_path(cx);
        let position = anchor.to_point_utf16(&buffer.snapshot());
        let timeout = ProjectSettings::get_global(cx)
            .global_lsp_settings
            .get_request_timeout();
        Some((server, abs_path, position, timeout))
    })?;
    let Some((server, abs_path, position, timeout)) = request else {
        return Ok(None);
    };
    let params = lsp::TextDocumentPositionParams {
        text_document: lsp::TextDocumentIdentifier {
            uri: file_path_to_lsp_url(&abs_path)?,
        },
        position: lsp::Position::new(position.row, position.column),
    };
    server
        .request::<PlainGoal>(params, timeout)
        .await
        .into_response()
        .context("$/lean/plainGoal")
}
