use std::sync::Arc;

use fs::FakeFs;
use futures::StreamExt as _;
use gpui::TestAppContext;
use language::{FakeLspAdapter, LanguageConfig, LanguageMatcher};
use project::{
    Project,
    lsp_store::lean_ext::{DependencyBuildMode, LeanDidOpen},
};
use serde_json::json;
use util::path;

use crate::init_test;

#[gpui::test]
async fn test_restart_lean_file_reopens_it_at_the_sent_version(cx: &mut TestAppContext) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree(path!("/dir"), json!({ "A.lean": "def a := 1" }))
        .await;
    let project = Project::test(fs, [path!("/dir").as_ref()], cx).await;
    let language_registry = project.read_with(cx, |project, _| project.languages().clone());
    language_registry.register_test_language(LanguageConfig {
        name: "Lean 4".into(),
        matcher: Arc::new(LanguageMatcher {
            path_suffixes: vec!["lean".into()],
            ..LanguageMatcher::default()
        }),
        ..LanguageConfig::default()
    });
    let mut fake_servers = language_registry.register_fake_lsp(
        "Lean 4",
        FakeLspAdapter {
            name: "lean4-lsp",
            ..FakeLspAdapter::default()
        },
    );
    let (buffer, _handle) = project
        .update(cx, |project, cx| {
            project.open_local_buffer_with_lsp(path!("/dir/A.lean"), cx)
        })
        .await
        .unwrap();
    cx.executor().run_until_parked();
    let mut server = fake_servers.next().await.unwrap();
    let uri = lsp::Uri::from_file_path(path!("/dir/A.lean")).unwrap();
    assert_eq!(
        server
            .receive_notification::<lsp::notification::DidOpenTextDocument>()
            .await
            .text_document
            .version,
        0
    );

    buffer.update(cx, |buffer, cx| buffer.edit([(10..10, "0")], None, cx));
    cx.executor().run_until_parked();
    let lsp_store = project.read_with(cx, |project, _| project.lsp_store());
    lsp_store
        .read_with(cx, |lsp_store, cx| {
            lsp_store.restart_lean_file(buffer.read(cx), cx)
        })
        .unwrap();

    assert_eq!(
        server
            .receive_notification::<lsp::notification::DidCloseTextDocument>()
            .await
            .text_document
            .uri,
        uri
    );
    let reopened = server.receive_notification::<LeanDidOpen>().await;
    assert_eq!(
        reopened.text_document,
        lsp::TextDocumentItem::new(uri.clone(), "lean 4".into(), 1, "def a := 10".into())
    );
    assert_eq!(reopened.dependency_build_mode, DependencyBuildMode::Once);

    buffer.update(cx, |buffer, cx| buffer.edit([(11..11, "0")], None, cx));
    cx.executor().run_until_parked();
    let changed = server
        .receive_notification::<lsp::notification::DidChangeTextDocument>()
        .await;
    assert_eq!(changed.text_document.version, 2);
}
