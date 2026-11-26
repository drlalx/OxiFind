#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use std::sync::mpsc;

use oxifind::engine::{IndexScope, SearchResponse, WorkerCommand, WorkerResponse};
use oxifind::search::SearchResult;
use oxifind::tui::app::{FocusedWidget, SearchOptions, TuiApp, TuiModal};
use oxifind::tui::ui;

fn press_key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn tui_app_initial_state() {
    let (tx, _rx_cmd) = mpsc::channel();
    let (_tx_resp, rx) = mpsc::channel();

    let app = TuiApp::with_channels(IndexScope::LocalDrives, tx, rx);

    assert_eq!(app.scope, IndexScope::LocalDrives);
    assert_eq!(app.query, "");
    assert_eq!(app.cursor, 0);
    assert!(app.is_indexing);
    assert_eq!(app.results.len(), 0);
    assert_eq!(app.focused, FocusedWidget::SearchInput);
    assert_eq!(app.active_modal, None);
    assert!(!app.should_quit);
}

#[test]
fn typing_triggers_search_command() {
    let (tx, rx_cmd) = mpsc::channel();
    let (_tx_resp, rx) = mpsc::channel();

    let mut app = TuiApp::with_channels(IndexScope::LocalDrives, tx, rx);

    app.handle_key_event(press_key(KeyCode::Char('f')));
    app.handle_key_event(press_key(KeyCode::Char('o')));
    app.handle_key_event(press_key(KeyCode::Char('o')));

    assert_eq!(app.query, "foo");
    assert_eq!(app.cursor, 3);

    // Verify commands were sent
    let mut last_cmd = None;
    while let Ok(cmd) = rx_cmd.try_recv() {
        last_cmd = Some(cmd);
    }

    match last_cmd.expect("command sent") {
        WorkerCommand::Search(req) => {
            assert_eq!(req.query, "foo");
            assert_eq!(req.revision, 3);
        }
        other => panic!("expected search command, got {other:?}"),
    }
}

#[test]
fn editing_keys_update_query_and_cursor() {
    let (tx, _rx_cmd) = mpsc::channel();
    let (_tx_resp, rx) = mpsc::channel();

    let mut app = TuiApp::with_channels(IndexScope::LocalDrives, tx, rx);

    app.handle_key_event(press_key(KeyCode::Char('a')));
    app.handle_key_event(press_key(KeyCode::Char('b')));
    app.handle_key_event(press_key(KeyCode::Char('c')));
    assert_eq!(app.query, "abc");
    assert_eq!(app.cursor, 3);

    // Left arrow
    app.handle_key_event(press_key(KeyCode::Left));
    assert_eq!(app.cursor, 2);

    // Backspace deletes 'b'
    app.handle_key_event(press_key(KeyCode::Backspace));
    assert_eq!(app.query, "ac");
    assert_eq!(app.cursor, 1);

    // Home / End
    app.handle_key_event(press_key(KeyCode::Home));
    assert_eq!(app.cursor, 0);
    app.handle_key_event(press_key(KeyCode::End));
    assert_eq!(app.cursor, 2);

    // Esc clears query
    app.handle_key_event(press_key(KeyCode::Esc));
    assert_eq!(app.query, "");
    assert_eq!(app.cursor, 0);
    assert!(!app.should_quit);

    // Esc on empty query exits
    app.handle_key_event(press_key(KeyCode::Esc));
    assert!(app.should_quit);
}

#[test]
fn selection_navigation_and_bounds() {
    let (tx, _rx_cmd) = mpsc::channel();
    let (_tx_resp, rx) = mpsc::channel();

    let mut app = TuiApp::with_channels(IndexScope::LocalDrives, tx, rx);

    app.results = vec![
        SearchResult {
            display: "one.rs".into(),
            path: "/path/one.rs".into(),
            path_resolved: true,
            attributes: 0,
        },
        SearchResult {
            display: "two.rs".into(),
            path: "/path/two.rs".into(),
            path_resolved: true,
            attributes: 0,
        },
        SearchResult {
            display: "three.rs".into(),
            path: "/path/three.rs".into(),
            path_resolved: true,
            attributes: 0,
        },
    ];
    app.table_state.select(Some(0));

    // Down
    app.handle_key_event(press_key(KeyCode::Down));
    assert_eq!(app.selected_index(), Some(1));

    app.handle_key_event(press_key(KeyCode::Down));
    assert_eq!(app.selected_index(), Some(2));

    // Bounded at end
    app.handle_key_event(press_key(KeyCode::Down));
    assert_eq!(app.selected_index(), Some(2));

    // Up
    app.handle_key_event(press_key(KeyCode::Up));
    assert_eq!(app.selected_index(), Some(1));

    // Home / End in Table focus
    app.focused = FocusedWidget::ResultTable;
    app.handle_key_event(press_key(KeyCode::End));
    assert_eq!(app.selected_index(), Some(2));
    app.handle_key_event(press_key(KeyCode::Home));
    assert_eq!(app.selected_index(), Some(0));

    // Vim keys in Table focus
    app.handle_key_event(press_key(KeyCode::Char('j')));
    assert_eq!(app.selected_index(), Some(1));
    app.handle_key_event(press_key(KeyCode::Char('k')));
    assert_eq!(app.selected_index(), Some(0));
}

#[test]
fn worker_responses_update_app_state() {
    let (tx, _rx_cmd) = mpsc::channel();
    let (tx_resp, rx) = mpsc::channel();

    let mut app = TuiApp::with_channels(IndexScope::LocalDrives, tx, rx);

    // Initial response
    tx_resp
        .send(WorkerResponse::IndexReady {
            indexed_items: 42,
            indexing_issues: 0,
            indexing_duration: std::time::Duration::from_millis(15),
            initial_results: vec![SearchResult {
                display: "hello.txt".into(),
                path: "/home/hello.txt".into(),
                path_resolved: true,
                attributes: 0,
            }],
        })
        .unwrap();

    app.poll_worker_responses();

    assert!(!app.is_indexing);
    assert_eq!(app.indexed_items, 42);
    assert_eq!(app.results.len(), 1);
    assert_eq!(app.selected_index(), Some(0));

    // Search response matching current revision
    app.latest_revision = 5;
    tx_resp
        .send(WorkerResponse::SearchResults(SearchResponse {
            revision: 5,
            results: vec![SearchResult {
                display: "matched.rs".into(),
                path: "/src/matched.rs".into(),
                path_resolved: true,
                attributes: 0,
            }],
        }))
        .unwrap();

    app.poll_worker_responses();
    assert_eq!(app.results[0].display, "matched.rs");
}

#[test]
fn modal_dialog_open_and_close() {
    let (tx, _rx_cmd) = mpsc::channel();
    let (_tx_resp, rx) = mpsc::channel();

    let mut app = TuiApp::with_channels(IndexScope::LocalDrives, tx, rx);

    // Open Help with F1
    app.handle_key_event(press_key(KeyCode::F(1)));
    assert_eq!(app.active_modal, Some(TuiModal::Help));

    // Esc closes Help
    app.handle_key_event(press_key(KeyCode::Esc));
    assert_eq!(app.active_modal, None);

    // Open ScopePicker with F2
    app.handle_key_event(press_key(KeyCode::F(2)));
    assert!(matches!(
        app.active_modal,
        Some(TuiModal::ScopePicker { .. })
    ));

    // Esc closes ScopePicker
    app.handle_key_event(press_key(KeyCode::Esc));
    assert_eq!(app.active_modal, None);
}

#[test]
fn search_options_toggles() {
    let mut opts = SearchOptions::default();
    assert_eq!(opts.apply_to_query("test"), "test");

    opts.match_case = true;
    assert_eq!(opts.apply_to_query("test"), "case: test");

    opts.whole_word = true;
    assert_eq!(opts.apply_to_query("test"), "case: word: test");

    opts.match_path = true;
    assert_eq!(opts.apply_to_query("test"), "case: word: path: test");

    opts.regex = true;
    assert_eq!(opts.apply_to_query("test"), "case: word: path: regex: test");
}

#[test]
fn renders_without_panic_on_test_backend() {
    let (tx, _rx_cmd) = mpsc::channel();
    let (_tx_resp, rx) = mpsc::channel();

    let mut app = TuiApp::with_channels(IndexScope::LocalDrives, tx, rx);
    app.results = vec![SearchResult {
        display: "test_file.rs".into(),
        path: "/dev/oxifind/test_file.rs".into(),
        path_resolved: true,
        attributes: 0x0000_0020,
    }];
    app.table_state.select(Some(0));

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|frame| {
            ui::render(frame, &mut app);
        })
        .expect("render succeeds");

    let buffer = terminal.backend().buffer().clone();
    let rendered_text = buffer
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(rendered_text.contains("OxiFind"));
    assert!(rendered_text.contains("Search"));
    assert!(rendered_text.contains("test_file.rs"));
    assert!(rendered_text.contains("Results"));

    // Render with Help modal open
    app.active_modal = Some(TuiModal::Help);
    terminal
        .draw(|frame| {
            ui::render(frame, &mut app);
        })
        .expect("render help succeeds");

    let buffer2 = terminal.backend().buffer().clone();
    let rendered2 = buffer2
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(rendered2.contains("Keyboard Shortcuts"));

    // Render with ScopePicker modal open
    app.active_modal = Some(TuiModal::ScopePicker {
        scopes: vec![IndexScope::LocalDrives, IndexScope::Root("/tmp".into())],
        selected: 0,
    });
    terminal
        .draw(|frame| {
            ui::render(frame, &mut app);
        })
        .expect("render scope picker succeeds");
}
