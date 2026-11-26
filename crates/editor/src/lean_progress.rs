//! Marks the lines that the Lean 4 server is still processing with bars in the gutter,
//! as vscode-lean4 does.

use std::{any::Any, ops::Range, sync::Arc};

use futures::{StreamExt as _, channel::mpsc};
use gpui::{App, Context, Entity, Task, Window};
use language::{Buffer, Point};
use lsp::{LanguageServer, LanguageServerId, notification::Notification as _};
use multi_buffer::{Anchor, MultiBufferSnapshot};
use project::{
    Project,
    lsp_command::file_path_to_lsp_url,
    lsp_store::lean_ext::{
        LEAN_SERVER_NAME, LeanFileProgress, LeanFileProgressKind, LeanFileProgressParams,
        LeanFileProgressProcessingInfo, observe_server_notifications,
    },
};
use text::Bias;
use theme::ActiveTheme as _;

use crate::{Addon, Editor};

enum LeanProcessing {}
enum LeanFatalError {}

/// The editor's tap on the Lean server that has its buffer open.
struct ProgressTap {
    server: LanguageServerId,
    _notifications: lsp::Subscription,
    _updates: Task<()>,
}

impl Addon for ProgressTap {
    fn to_any(&self) -> &dyn Any {
        self
    }
}

pub(crate) fn init(cx: &mut App) {
    cx.observe_new(|editor: &mut Editor, window, cx| {
        let Some(window) = window else {
            return;
        };
        let Some(project) = editor.project.clone().filter(|_| editor.mode().is_full()) else {
            return;
        };
        // A buffer shown in another editor already is registered with its servers.
        if let Some(buffer) = editor.buffer().read(cx).as_singleton()
            && let Some(server) = registered_lean_server(&project, &buffer, cx)
        {
            follow(editor, server, buffer, window, cx);
        }
        cx.subscribe_in(
            &project,
            window,
            |editor, project, event, window, cx| match event {
                project::Event::LanguageServerBufferRegistered {
                    server_id,
                    buffer_id,
                    ..
                } => {
                    let buffer = editor
                        .buffer()
                        .read(cx)
                        .as_singleton()
                        .filter(|buffer| buffer.read(cx).remote_id() == *buffer_id);
                    let server = project
                        .read(cx)
                        .lsp_store()
                        .read(cx)
                        .language_server_for_id(*server_id)
                        .filter(|server| server.name() == LEAN_SERVER_NAME);
                    if let (Some(buffer), Some(server)) = (buffer, server) {
                        follow(editor, server, buffer, window, cx);
                    }
                }
                project::Event::LanguageServerRemoved(server_id)
                    if editor
                        .addon::<ProgressTap>()
                        .is_some_and(|tap| tap.server == *server_id) =>
                {
                    editor.unregister_addon::<ProgressTap>();
                    show(editor, &[], cx);
                }
                _ => {}
            },
        )
        .detach();
    })
    .detach();
}

fn registered_lean_server(
    project: &Entity<Project>,
    buffer: &Entity<Buffer>,
    cx: &App,
) -> Option<Arc<LanguageServer>> {
    let lsp_store = project.read(cx).lsp_store().read(cx);
    lsp_store
        .language_server_ids_for_opened_buffer(buffer.read(cx).remote_id())?
        .iter()
        .filter_map(|id| lsp_store.language_server_for_id(*id))
        .find(|server| server.name() == LEAN_SERVER_NAME)
}

fn follow(
    editor: &mut Editor,
    server: Arc<LanguageServer>,
    buffer: Entity<Buffer>,
    window: &mut Window,
    cx: &mut Context<Editor>,
) {
    if editor
        .addon::<ProgressTap>()
        .is_some_and(|tap| tap.server == server.server_id())
    {
        return;
    }
    let Some(uri) = buffer
        .read(cx)
        .file()
        .and_then(|file| file.as_local())
        .and_then(|file| file_path_to_lsp_url(&file.abs_path(cx)).ok())
    else {
        return;
    };
    let (progress_tx, mut progress_rx) = mpsc::unbounded();
    let notifications =
        observe_server_notifications(&server, LeanFileProgress::METHOD, move |params| {
            match serde_json::from_str::<LeanFileProgressParams>(params.get()) {
                Ok(params) if params.text_document.uri == uri => {
                    progress_tx.unbounded_send(params.processing).ok();
                }
                Ok(_) => {}
                Err(error) => log::warn!("unexpected Lean file progress: {error}"),
            }
        });
    let updates = cx.spawn_in(window, async move |editor, cx| {
        while let Some(mut processing) = progress_rx.next().await {
            while let Ok(newer) = progress_rx.try_recv() {
                processing = newer;
            }
            if editor
                .update(cx, |editor, cx| show(editor, &processing, cx))
                .is_err()
            {
                break;
            }
        }
    });
    editor.register_addon(ProgressTap {
        server: server.server_id(),
        _notifications: notifications,
        _updates: updates,
    });
}

fn show(
    editor: &mut Editor,
    processing: &[LeanFileProgressProcessingInfo],
    cx: &mut Context<Editor>,
) {
    let snapshot = editor.buffer().read(cx).snapshot(cx);
    let lines = |kind| -> Vec<Range<Anchor>> {
        processing
            .iter()
            .filter(|info| info.kind == kind)
            .map(|info| line_range(&snapshot, info.range))
            .collect()
    };
    let processing = lines(LeanFileProgressKind::Processing);
    let fatal_errors = lines(LeanFileProgressKind::FatalError);
    editor.highlight_gutter::<LeanProcessing>(processing, |cx| cx.theme().status().warning, cx);
    editor.highlight_gutter::<LeanFatalError>(fatal_errors, |cx| cx.theme().status().error, cx);
}

/// The whole lines that an LSP range touches.
fn line_range(snapshot: &MultiBufferSnapshot, range: lsp::Range) -> Range<Anchor> {
    let start = snapshot.clip_point(Point::new(range.start.line, 0), Bias::Left);
    let end = snapshot.clip_point(Point::new(range.end.line, 0), Bias::Left);
    snapshot.anchor_before(start)..snapshot.anchor_after(end)
}

#[cfg(test)]
mod tests {
    use gpui::{Hsla, TestAppContext};
    use multi_buffer::MultiBufferOffset;

    use super::*;
    use crate::{
        lean_ext::lean_lsp_test_context, test::editor_lsp_test_context::EditorLspTestContext,
    };

    fn progress(uri: &lsp::Uri, lines: &[(u32, u32, u8)]) -> LeanFileProgressParams {
        LeanFileProgressParams {
            text_document: lsp::VersionedTextDocumentIdentifier::new(uri.clone(), 1),
            processing: lines
                .iter()
                .map(|&(start, end, kind)| LeanFileProgressProcessingInfo {
                    range: lsp::Range::new(
                        lsp::Position::new(start, 0),
                        lsp::Position::new(end, 0),
                    ),
                    kind: kind.into(),
                })
                .collect(),
        }
    }

    fn gutter_bars(cx: &mut EditorLspTestContext) -> Vec<(Range<u32>, Hsla)> {
        cx.update_editor(|editor, window, cx| {
            let snapshot = editor.snapshot(window, cx);
            let buffer = snapshot.buffer_snapshot();
            let all = buffer.anchor_before(MultiBufferOffset(0))..buffer.anchor_after(buffer.len());
            editor
                .gutter_highlights_in_range(all, &snapshot.display_snapshot, cx)
                .into_iter()
                .map(|(range, color)| (range.start.row().0..range.end.row().0, color))
                .collect()
        })
    }

    #[gpui::test]
    async fn marks_the_lines_the_server_is_processing(cx: &mut TestAppContext) {
        let mut cx = lean_lsp_test_context(Default::default(), cx).await;
        cx.set_state("ˇa\nb\nc\nd\ne");
        cx.run_until_parked();
        let (warning, error) =
            cx.update(|_, cx| (cx.theme().status().warning, cx.theme().status().error));
        let uri = cx.buffer_lsp_url.clone();

        cx.lsp
            .notify::<LeanFileProgress>(progress(&uri, &[(1, 2, 1), (4, 9, 2)]));
        cx.run_until_parked();
        assert_eq!(gutter_bars(&mut cx), [(1..2, warning), (4..4, error)]);

        let other = lsp::Uri::from_file_path(util::path!("/root/dir/Other.lean")).unwrap();
        cx.lsp
            .notify::<LeanFileProgress>(progress(&other, &[(0, 4, 1)]));
        cx.run_until_parked();
        assert_eq!(gutter_bars(&mut cx), [(1..2, warning), (4..4, error)]);

        cx.lsp.notify::<LeanFileProgress>(progress(&uri, &[]));
        cx.run_until_parked();
        assert_eq!(gutter_bars(&mut cx), []);
    }
}
