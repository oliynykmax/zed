use std::{sync::Arc, time::Duration};

use anyhow::Context as _;
use futures::{StreamExt as _, channel::mpsc};
use gpui::{
    App, AppContext as _, AsyncWindowContext, Context, Entity, TaskExt, WeakEntity, Window, actions,
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
use workspace::{SplitDirection, notifications::NotifyResultExt as _};

use crate::{
    Editor, EditorEvent, ShowLeanGoal, element::register_action,
    lsp_ext::find_specific_language_server_in_selection,
};

actions!(
    lean,
    [
        /// Makes the Lean server elaborate the current file from scratch, building its
        /// outdated imports first.
        RestartFile
    ]
);

const REFRESH_DEBOUNCE: Duration = Duration::from_millis(100);

pub(crate) fn is_lean_language(language: &Language) -> bool {
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
        register_action(editor, window, crate::lean_input::convert);
        register_action(editor, window, restart_file);
    }
}

fn restart_file(editor: &mut Editor, _: &RestartFile, _: &mut Window, cx: &mut Context<Editor>) {
    let (Some(project), Some(workspace)) = (editor.project.clone(), editor.workspace()) else {
        return;
    };
    let head = editor.selections.newest_anchor().head();
    let Some((buffer, _)) = editor.buffer().read(cx).text_anchor_for_position(head, cx) else {
        return;
    };
    if !buffer
        .read(cx)
        .language()
        .is_some_and(|language| is_lean_language(language))
    {
        return;
    }
    let restarted = project
        .read(cx)
        .lsp_store()
        .read(cx)
        .restart_lean_file(buffer.read(cx), cx);
    workspace.update(cx, |workspace, cx| {
        restarted.notify_err(workspace, cx);
    });
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

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;
    use language::{FakeLspAdapter, LanguageConfig, LanguageMatcher};
    use project::lsp_store::lean_ext::{DependencyBuildMode, LeanDidOpen};

    use super::*;
    use crate::{editor_tests::init_test, test::editor_lsp_test_context::EditorLspTestContext};

    #[gpui::test]
    async fn restart_file_reopens_the_document_in_the_lean_server(cx: &mut TestAppContext) {
        init_test(cx, |_| {});
        let lean = Language::new(
            LanguageConfig {
                name: "Lean 4".into(),
                matcher: LanguageMatcher {
                    path_suffixes: vec!["lean".into()],
                    ..Default::default()
                }
                .into(),
                ..LanguageConfig::default()
            },
            None,
        );
        let mut cx = EditorLspTestContext::new_with_adapter(
            lean,
            FakeLspAdapter {
                name: "lean4-lsp",
                ..FakeLspAdapter::default()
            },
            cx,
        )
        .await;
        cx.set_state("theorem t : True := ˇtrivial");
        cx.run_until_parked();

        cx.dispatch_action(RestartFile);
        let closed = cx
            .lsp
            .receive_notification::<lsp::notification::DidCloseTextDocument>()
            .await;
        assert_eq!(closed.text_document.uri, cx.buffer_lsp_url);
        let reopened = cx.lsp.receive_notification::<LeanDidOpen>().await;
        assert_eq!(reopened.text_document.uri, cx.buffer_lsp_url);
        assert_eq!(reopened.text_document.text, "theorem t : True := trivial");
        assert_eq!(reopened.dependency_build_mode, DependencyBuildMode::Once);
    }
}
