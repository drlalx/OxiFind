use std::{
    error::Error,
    sync::mpsc,
    time::{Duration, Instant},
};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::widgets::TableState;

use crate::{
    engine::{
        IndexScope, ResultSort, SearchRequest, SearchResponse, SortColumn,
        WorkerCommand, WorkerResponse, effective_query, spawn_search_worker,
    },
    platform::{OpenAction, open_path},
    search::SearchResult,
};

use super::terminal::TerminalGuard;
use super::ui;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SearchOptions {
    pub match_case: bool,
    pub whole_word: bool,
    pub match_path: bool,
    pub regex: bool,
}

impl SearchOptions {
    pub fn apply_to_query(&self, raw: &str) -> String {
        let mut prefix = String::new();
        if self.match_case {
            prefix.push_str("case: ");
        }
        if self.whole_word {
            prefix.push_str("word: ");
        }
        if self.match_path {
            prefix.push_str("path: ");
        }
        if self.regex {
            prefix.push_str("regex: ");
        }
        format!("{prefix}{raw}")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TuiModal {
    Help,
    ScopePicker {
        scopes: Vec<IndexScope>,
        selected: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FocusedWidget {
    SearchInput,
    ResultTable,
}

pub struct TuiApp {
    pub scope: IndexScope,
    pub query: String,
    pub cursor: usize,
    pub options: SearchOptions,
    pub sort: ResultSort,
    pub table_state: TableState,
    pub results: Vec<SearchResult>,
    pub focused: FocusedWidget,

    pub command_tx: mpsc::Sender<WorkerCommand>,
    pub response_rx: mpsc::Receiver<WorkerResponse>,
    pub latest_revision: u64,

    pub is_indexing: bool,
    pub indexed_items: usize,
    pub indexing_issues: usize,
    pub search_duration_micros: u128,
    pub status_message: Option<(String, Instant)>,
    pub active_modal: Option<TuiModal>,
    pub should_quit: bool,
}

impl TuiApp {
    pub fn new(scope: IndexScope) -> Self {
        let (command_tx, command_rx) = mpsc::channel::<WorkerCommand>();
        let (response_tx, response_rx) = mpsc::channel::<WorkerResponse>();

        spawn_search_worker(
            scope.clone(),
            command_tx.clone(),
            command_rx,
            response_tx,
        );

        Self::with_channels(scope, command_tx, response_rx)
    }

    pub fn with_channels(
        scope: IndexScope,
        command_tx: mpsc::Sender<WorkerCommand>,
        response_rx: mpsc::Receiver<WorkerResponse>,
    ) -> Self {
        let mut table_state = TableState::default();
        table_state.select(Some(0));

        Self {
            scope,
            query: String::new(),
            cursor: 0,
            options: SearchOptions::default(),
            sort: ResultSort::default(),
            table_state,
            results: Vec::new(),
            focused: FocusedWidget::SearchInput,

            command_tx,
            response_rx,
            latest_revision: 0,

            is_indexing: true,
            indexed_items: 0,
            indexing_issues: 0,
            search_duration_micros: 0,
            status_message: None,
            active_modal: None,
            should_quit: false,
        }
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.table_state.selected()
    }

    pub fn selected_result(&self) -> Option<&SearchResult> {
        let idx = self.table_state.selected()?;
        self.results.get(idx)
    }

    pub fn effective_search_query(&self) -> String {
        let opt_query = self.options.apply_to_query(&self.query);
        effective_query(&opt_query, self.sort)
    }

    pub fn trigger_search(&mut self) {
        self.latest_revision = self.latest_revision.wrapping_add(1);
        let request = SearchRequest {
            revision: self.latest_revision,
            query: self.effective_search_query(),
        };
        let _ = self.command_tx.send(WorkerCommand::Search(request));
    }

    pub fn select_next(&mut self) {
        if self.results.is_empty() {
            self.table_state.select(None);
            return;
        }
        let next = match self.table_state.selected() {
            Some(i) if i + 1 < self.results.len() => i + 1,
            Some(_) => self.results.len().saturating_sub(1),
            None => 0,
        };
        self.table_state.select(Some(next));
    }

    pub fn select_prev(&mut self) {
        if self.results.is_empty() {
            self.table_state.select(None);
            return;
        }
        let prev = match self.table_state.selected() {
            Some(i) if i > 0 => i - 1,
            _ => 0,
        };
        self.table_state.select(Some(prev));
    }

    pub fn select_page_down(&mut self, page_size: usize) {
        if self.results.is_empty() {
            return;
        }
        let current = self.table_state.selected().unwrap_or(0);
        let next = (current + page_size).min(self.results.len().saturating_sub(1));
        self.table_state.select(Some(next));
    }

    pub fn select_page_up(&mut self, page_size: usize) {
        if self.results.is_empty() {
            return;
        }
        let current = self.table_state.selected().unwrap_or(0);
        let next = current.saturating_sub(page_size);
        self.table_state.select(Some(next));
    }

    pub fn select_first(&mut self) {
        if !self.results.is_empty() {
            self.table_state.select(Some(0));
        }
    }

    pub fn select_last(&mut self) {
        if !self.results.is_empty() {
            self.table_state.select(Some(self.results.len() - 1));
        }
    }

    pub fn toggle_sort(&mut self, column: SortColumn) {
        self.sort = self.sort.toggled(column);
        self.trigger_search();
    }

    pub fn open_selected(&mut self) {
        if let Some(result) = self.selected_result() {
            let path = std::path::Path::new(&result.path);
            match open_path(path, OpenAction::Open) {
                Ok(()) => {
                    self.set_status(format!("Opened: {}", result.display));
                }
                Err(err) => {
                    self.set_status(format!("Failed to open: {err}"));
                }
            }
        }
    }

    pub fn reveal_selected(&mut self) {
        if let Some(result) = self.selected_result() {
            let path = std::path::Path::new(&result.path);
            match open_path(path, OpenAction::Reveal) {
                Ok(()) => {
                    self.set_status(format!("Revealed: {}", result.display));
                }
                Err(err) => {
                    self.set_status(format!("Failed to reveal: {err}"));
                }
            }
        }
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_message = Some((msg.into(), Instant::now()));
    }

    pub fn poll_worker_responses(&mut self) {
        while let Ok(response) = self.response_rx.try_recv() {
            match response {
                WorkerResponse::IndexReady {
                    indexed_items,
                    indexing_issues,
                    indexing_duration: _,
                    initial_results,
                } => {
                    self.is_indexing = false;
                    self.indexed_items = indexed_items;
                    self.indexing_issues = indexing_issues;
                    if self.query.is_empty() {
                        self.results = initial_results;
                        if !self.results.is_empty() && self.table_state.selected().is_none() {
                            self.table_state.select(Some(0));
                        }
                    }
                }
                WorkerResponse::SearchResults(SearchResponse { revision, results }) => {
                    if revision == self.latest_revision {
                        self.results = results;
                        if self.results.is_empty() {
                            self.table_state.select(None);
                        } else {
                            let curr = self.table_state.selected().unwrap_or(0);
                            let bounded = curr.min(self.results.len().saturating_sub(1));
                            self.table_state.select(Some(bounded));
                        }
                    }
                }
                WorkerResponse::IndexingFailed(error) => {
                    self.is_indexing = false;
                    self.set_status(format!("Indexing failed: {error}"));
                }
                WorkerResponse::IndexUpdated {
                    indexed_items,
                    indexing_issues,
                    refreshed,
                } => {
                    self.indexed_items = indexed_items;
                    self.indexing_issues = indexing_issues;
                    if refreshed.revision == self.latest_revision {
                        self.results = refreshed.results;
                    }
                }
                WorkerResponse::UpdateFailed(error) => {
                    self.set_status(format!("Live update warning: {error}"));
                }
            }
        }

        // Clear status message after 4 seconds
        if self
            .status_message
            .as_ref()
            .is_some_and(|(_, time)| time.elapsed() > Duration::from_secs(4))
        {
            self.status_message = None;
        }
    }

    pub fn handle_key_event(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }

        // Handle Active Modal
        if let Some(modal) = &mut self.active_modal {
            match modal {
                TuiModal::Help => {
                    if key.code == KeyCode::Esc || key.code == KeyCode::Char('q') || key.code == KeyCode::F(1) {
                        self.active_modal = None;
                    }
                }
                TuiModal::ScopePicker { scopes, selected } => match key.code {
                    KeyCode::Esc => {
                        self.active_modal = None;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if *selected > 0 {
                            *selected -= 1;
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if *selected + 1 < scopes.len() {
                            *selected += 1;
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(chosen) = scopes.get(*selected).cloned() {
                            self.active_modal = None;
                            self.restart_with_scope(chosen);
                        }
                    }
                    _ => {}
                },
            }
            return;
        }

        // Global Hotkeys
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('c') | KeyCode::Char('q') => {
                    self.should_quit = true;
                    return;
                }
                KeyCode::Char('o') => {
                    self.reveal_selected();
                    return;
                }
                KeyCode::Char('r') => {
                    self.restart_with_scope(self.scope.clone());
                    return;
                }
                _ => {}
            }
        }

        if key.modifiers.contains(KeyModifiers::ALT) {
            match key.code {
                KeyCode::Char('1') => self.toggle_sort(SortColumn::Relevance),
                KeyCode::Char('2') => self.toggle_sort(SortColumn::Name),
                KeyCode::Char('3') => self.toggle_sort(SortColumn::Path),
                KeyCode::Char('4') => self.toggle_sort(SortColumn::Type),
                KeyCode::Char('c') => {
                    self.options.match_case = !self.options.match_case;
                    self.trigger_search();
                }
                KeyCode::Char('w') => {
                    self.options.whole_word = !self.options.whole_word;
                    self.trigger_search();
                }
                KeyCode::Char('p') => {
                    self.options.match_path = !self.options.match_path;
                    self.trigger_search();
                }
                KeyCode::Char('r') => {
                    self.options.regex = !self.options.regex;
                    self.trigger_search();
                }
                KeyCode::Char('o') => self.reveal_selected(),
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::F(1) => {
                self.active_modal = Some(TuiModal::Help);
            }
            KeyCode::F(2) => {
                self.open_scope_picker();
            }
            KeyCode::F(5) => {
                self.restart_with_scope(self.scope.clone());
            }
            KeyCode::Tab => {
                self.focused = match self.focused {
                    FocusedWidget::SearchInput => FocusedWidget::ResultTable,
                    FocusedWidget::ResultTable => FocusedWidget::SearchInput,
                };
            }
            KeyCode::Enter => {
                self.open_selected();
            }
            KeyCode::Down => self.select_next(),
            KeyCode::Up => self.select_prev(),
            KeyCode::PageDown => self.select_page_down(15),
            KeyCode::PageUp => self.select_page_up(15),
            KeyCode::Home if self.focused == FocusedWidget::ResultTable => self.select_first(),
            KeyCode::End if self.focused == FocusedWidget::ResultTable => self.select_last(),

            // When ResultTable is focused, allow vim keys j/k
            KeyCode::Char('j') if self.focused == FocusedWidget::ResultTable => self.select_next(),
            KeyCode::Char('k') if self.focused == FocusedWidget::ResultTable => self.select_prev(),
            KeyCode::Char('q') if self.focused == FocusedWidget::ResultTable => self.should_quit = true,
            KeyCode::Char('?') if self.focused == FocusedWidget::ResultTable => {
                self.active_modal = Some(TuiModal::Help);
            }

            // Search Input Editing
            KeyCode::Char(c) => {
                self.query.insert(self.cursor, c);
                self.cursor += 1;
                self.trigger_search();
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.query.remove(self.cursor);
                    self.trigger_search();
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.query.len() {
                    self.query.remove(self.cursor);
                    self.trigger_search();
                }
            }
            KeyCode::Left => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
            }
            KeyCode::Right => {
                if self.cursor < self.query.len() {
                    self.cursor += 1;
                }
            }
            KeyCode::Home => {
                self.cursor = 0;
            }
            KeyCode::End => {
                self.cursor = self.query.len();
            }
            KeyCode::Esc => {
                if !self.query.is_empty() {
                    self.query.clear();
                    self.cursor = 0;
                    self.trigger_search();
                } else {
                    self.should_quit = true;
                }
            }
            _ => {}
        }
    }

    pub fn open_scope_picker(&mut self) {
        let mut scopes = vec![IndexScope::LocalDrives];
        #[cfg(unix)]
        {
            scopes.push(IndexScope::Root(std::path::PathBuf::from("/")));
            if let Ok(home) = std::env::var("HOME") {
                scopes.push(IndexScope::Root(std::path::PathBuf::from(home)));
            }
        }
        #[cfg(windows)]
        {
            for drive in b'C'..=b'Z' {
                let path = format!("{}:\\", drive as char);
                if std::path::Path::new(&path).exists() {
                    scopes.push(IndexScope::Root(std::path::PathBuf::from(path)));
                }
            }
        }

        let selected = scopes.iter().position(|s| s == &self.scope).unwrap_or(0);
        self.active_modal = Some(TuiModal::ScopePicker { scopes, selected });
    }

    pub fn restart_with_scope(&mut self, scope: IndexScope) {
        self.scope = scope.clone();
        let (command_tx, command_rx) = mpsc::channel::<WorkerCommand>();
        let (response_tx, response_rx) = mpsc::channel::<WorkerResponse>();

        spawn_search_worker(scope, command_tx.clone(), command_rx, response_tx);

        self.command_tx = command_tx;
        self.response_rx = response_rx;
        self.is_indexing = true;
        self.results.clear();
        self.table_state.select(None);
        self.set_status("Re-indexing started...");
        self.trigger_search();
    }

    pub fn run(&mut self, terminal: &mut TerminalGuard) -> Result<(), Box<dyn Error + Send + Sync>> {
        let tick_rate = Duration::from_millis(50);
        let mut last_tick = Instant::now();

        while !self.should_quit {
            self.poll_worker_responses();

            terminal.draw(|frame| {
                ui::render(frame, self);
            })?;

            let timeout = tick_rate
                .checked_sub(last_tick.elapsed())
                .unwrap_or_else(|| Duration::from_millis(0));

            if event::poll(timeout)? && let Event::Key(key) = event::read()? {
                self.handle_key_event(key);
            }

            if last_tick.elapsed() >= tick_rate {
                last_tick = Instant::now();
            }
        }

        Ok(())
    }
}
