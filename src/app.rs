use std::{
    error::Error,
    ffi::OsString,
    fmt, io,
    ops::ControlFlow,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use eframe::egui;
use egui_extras::{Column, TableBuilder};

#[cfg(windows)]
use crate::ntfs::{NtfsMftSource, NtfsMultiVolumeSource, local_fixed_volumes};
use crate::{
    crawler::DirectorySource,
    platform::{self, OpenAction, open_path},
    search::{IndexSource, JournalCheckpoint, SearchIndex, SearchResult, UpdateBatch},
};

const RESULT_LIMIT: usize = 200;
const UPDATE_POLL_INTERVAL: Duration = Duration::from_secs(1);

const FILE_ATTRIBUTE_READONLY: u32 = 0x0000_0001;
const FILE_ATTRIBUTE_HIDDEN: u32 = 0x0000_0002;
const FILE_ATTRIBUTE_SYSTEM: u32 = 0x0000_0004;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;

const APP_NAME: &str = "OxiFind";
const SEARCH_FIELD_ID: &str = "search_field";

const SEARCH_SYNTAX: [(&str, &str); 7] = [
    ("text", "File name or path contains text"),
    ("a b", "Every term must match"),
    ("^text", "File name starts with text"),
    ("=text", "File name is exactly text"),
    ("path:text", "Full path contains text"),
    ("\"a b\"", "Quotes keep spaces inside a term"),
    (
        "sort:name|path|type|relevance",
        "Orders results; a leading - reverses the order",
    ),
];

/// One result as shown in the table: file name, containing folder, and type.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct ResultRow {
    name: String,
    folder: String,
    kind: String,
    is_folder: bool,
    path: String,
    path_resolved: bool,
}

impl ResultRow {
    fn from_result(result: SearchResult) -> Self {
        let (name, folder) = split_result_path(&result.path);
        Self {
            name,
            folder,
            kind: describe_attributes(result.attributes),
            is_folder: result.attributes & FILE_ATTRIBUTE_DIRECTORY != 0,
            path: result.path,
            path_resolved: result.path_resolved,
        }
    }

    fn action_path(&self) -> io::Result<PathBuf> {
        if !self.path_resolved {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the indexed path could not be resolved",
            ));
        }
        std::path::absolute(&self.path)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum SortColumn {
    #[default]
    Relevance,
    Name,
    Path,
    Type,
}

impl SortColumn {
    const ALL: [Self; 4] = [Self::Relevance, Self::Name, Self::Path, Self::Type];

    fn label(self) -> &'static str {
        match self {
            Self::Relevance => "Relevance",
            Self::Name => "Name",
            Self::Path => "Path",
            Self::Type => "Type",
        }
    }

    fn query_key(self) -> &'static str {
        match self {
            Self::Relevance => "relevance",
            Self::Name => "name",
            Self::Path => "path",
            Self::Type => "type",
        }
    }
}

/// The sort order chosen through column headers or the View menu.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ResultSort {
    column: SortColumn,
    descending: bool,
}

impl ResultSort {
    /// Clicking the active column reverses it; any other column starts ascending.
    fn toggled(self, column: SortColumn) -> Self {
        Self {
            column,
            descending: self.column == column && !self.descending,
        }
    }

    fn header_label(self, column: SortColumn) -> String {
        match (self.column == column, self.descending) {
            (true, false) => format!("{} \u{2B06}", column.label()),
            (true, true) => format!("{} \u{2B07}", column.label()),
            (false, _) => column.label().to_owned(),
        }
    }

    fn status_label(self) -> String {
        let direction = if self.descending {
            "descending"
        } else {
            "ascending"
        };
        format!("Sort: {} ({direction})", self.column.label())
    }
}

/// Prefixes the UI sort order, so a `sort:` term typed into the query still wins.
fn effective_query(query: &str, sort: ResultSort) -> String {
    if sort == ResultSort::default() {
        return query.to_owned();
    }
    let direction = if sort.descending { "-" } else { "" };
    format!("sort:{direction}{} {query}", sort.column.query_key())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RowAction {
    Open(OpenAction),
    CopyName,
    CopyPath,
}

#[derive(Default)]
struct SearchRequest {
    revision: u64,
    query: String,
}

#[derive(Default)]
struct SearchResponse {
    revision: u64,
    results: Vec<SearchResult>,
}

enum WorkerCommand {
    Search(SearchRequest),
    ApplyUpdates(UpdateBatch),
    UpdateFailed(String),
}

enum WorkerResponse {
    IndexReady {
        indexed_items: usize,
        indexing_issues: usize,
        indexing_duration: Duration,
        initial_results: Vec<SearchResult>,
    },
    SearchResults(SearchResponse),
    IndexingFailed(String),
    /// Published after a batch was applied, with the latest query re-run against the new snapshot.
    IndexUpdated {
        indexed_items: usize,
        indexing_issues: usize,
        refreshed: SearchResponse,
    },
    UpdateFailed(String),
}

pub fn run(scope: IndexScope) -> Result<(), Box<dyn Error + Send + Sync>> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_inner_size([1100.0, 700.0])
            .with_min_inner_size([520.0, 320.0]),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|creation| Ok(Box::new(SearchApp::new(scope, creation.egui_ctx.clone())))),
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub use crate::engine::IndexScope;

/// Sends worker responses and wakes the UI, so it only repaints when something changed.
struct ResponseSender {
    tx: mpsc::Sender<WorkerResponse>,
    ctx: egui::Context,
}

impl Default for ResponseSender {
    fn default() -> Self {
        Self {
            tx: mpsc::channel().0,
            ctx: egui::Context::default(),
        }
    }
}

impl ResponseSender {
    fn send(&self, response: WorkerResponse) -> Result<(), mpsc::SendError<WorkerResponse>> {
        self.tx.send(response)?;
        self.ctx.request_repaint();
        Ok(())
    }
}

struct SearchApp {
    scope: IndexScope,
    request_tx: mpsc::Sender<WorkerCommand>,
    response_rx: mpsc::Receiver<WorkerResponse>,
    query: String,
    sort: ResultSort,
    rows: Vec<ResultRow>,
    selected: Option<usize>,
    latest_revision: u64,
    is_indexing: bool,
    indexing_failed: bool,
    is_elevated: bool,
    worker_stopped: bool,
    index_status: String,
    notice: Option<String>,
    show_status_bar: bool,
    show_syntax_help: bool,
    show_about: bool,
    focus_search: bool,
}

/// An app without a worker; `SearchApp::new` connects one.
impl Default for SearchApp {
    fn default() -> Self {
        Self {
            scope: IndexScope::Root(PathBuf::new()),
            request_tx: mpsc::channel().0,
            response_rx: mpsc::channel().1,
            query: String::new(),
            sort: ResultSort::default(),
            rows: Vec::new(),
            selected: None,
            latest_revision: 0,
            is_indexing: false,
            indexing_failed: false,
            is_elevated: false,
            worker_stopped: true,
            index_status: String::new(),
            notice: None,
            show_status_bar: true,
            show_syntax_help: false,
            show_about: false,
            focus_search: true,
        }
    }
}

impl SearchApp {
    fn new(scope: IndexScope, ctx: egui::Context) -> Self {
        let (request_tx, request_rx) = mpsc::channel::<WorkerCommand>();
        let (response_tx, response_rx) = mpsc::channel::<WorkerResponse>();
        spawn_search_worker(
            scope.clone(),
            request_tx.clone(),
            request_rx,
            ResponseSender {
                tx: response_tx,
                ctx,
            },
        );

        Self {
            index_status: format_indexing_status(&scope),
            scope,
            request_tx,
            response_rx,
            is_indexing: true,
            is_elevated: platform::is_elevated(),
            worker_stopped: false,
            ..Self::default()
        }
    }

    fn drain_responses(&mut self) {
        loop {
            match self.response_rx.try_recv() {
                Ok(response) => self.apply_response(response),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.mark_worker_stopped();
                    break;
                }
            }
        }
    }

    fn apply_response(&mut self, response: WorkerResponse) {
        match response {
            WorkerResponse::IndexReady {
                indexed_items,
                indexing_issues,
                indexing_duration,
                initial_results,
            } => {
                self.is_indexing = false;
                self.index_status = format_index_status(
                    &self.scope,
                    indexing_duration,
                    indexed_items,
                    indexing_issues,
                );
                // Queries typed during indexing are answered right after this response.
                if self.latest_revision == 0 {
                    self.set_rows(initial_results);
                }
            }
            WorkerResponse::SearchResults(response) => {
                if response.revision == self.latest_revision {
                    self.set_rows(response.results);
                }
            }
            WorkerResponse::IndexingFailed(error) => {
                self.is_indexing = false;
                self.indexing_failed = true;
                self.worker_stopped = true;
                self.index_status = format!("Indexing failed: {error}");
            }
            WorkerResponse::IndexUpdated {
                indexed_items,
                indexing_issues,
                refreshed,
            } => {
                // A newer query's own response will follow, already based on this snapshot.
                if refreshed.revision == self.latest_revision {
                    self.set_rows(refreshed.results);
                }
                self.index_status = format_update_status(indexed_items, indexing_issues);
            }
            WorkerResponse::UpdateFailed(message) => self.notice = Some(message),
        }
    }

    /// Replaces the rows and keeps the selection on the same path when it is still listed.
    fn set_rows(&mut self, results: Vec<SearchResult>) {
        let selected_path = self
            .selected
            .and_then(|index| self.rows.get(index))
            .map(|row| row.path.clone());
        self.rows = results.into_iter().map(ResultRow::from_result).collect();
        self.selected =
            selected_path.and_then(|path| self.rows.iter().position(|row| row.path == path));
    }

    fn mark_worker_stopped(&mut self) {
        if !self.worker_stopped {
            self.worker_stopped = true;
            self.is_indexing = false;
            self.index_status = "Search worker stopped".to_owned();
        }
    }

    fn send_search(&mut self) {
        self.latest_revision = self.latest_revision.saturating_add(1);
        self.notice = None;
        if self.worker_stopped {
            return;
        }

        let request = SearchRequest {
            revision: self.latest_revision,
            query: effective_query(&self.query, self.sort),
        };
        if self
            .request_tx
            .send(WorkerCommand::Search(request))
            .is_err()
        {
            self.mark_worker_stopped();
        }
    }

    fn set_sort(&mut self, sort: ResultSort) {
        if sort != self.sort {
            self.sort = sort;
            self.send_search();
        }
    }

    fn clear_search(&mut self) {
        if !self.query.is_empty() {
            self.query.clear();
            self.send_search();
        }
        self.focus_search = true;
    }

    fn selected_row(&self) -> Option<usize> {
        self.selected.filter(|&index| index < self.rows.len())
    }

    fn run_row_action(&mut self, ctx: &egui::Context, index: usize, action: RowAction) {
        let Some(row) = self.rows.get(index) else {
            return;
        };
        match action {
            RowAction::Open(open) => {
                if let Err(error) = row.action_path().and_then(|path| open_path(&path, open)) {
                    self.notice = Some(format!("Could not open {}: {error}", row.path));
                }
            }
            RowAction::CopyName => ctx.copy_text(row.name.clone()),
            RowAction::CopyPath => ctx.copy_text(row.path.clone()),
        }
    }

    fn run_selected_action(&mut self, ctx: &egui::Context, action: RowAction) {
        if let Some(index) = self.selected_row() {
            self.run_row_action(ctx, index, action);
        }
    }

    fn restart_as_administrator(&mut self, ctx: &egui::Context) {
        let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
        match platform::restart_elevated(&arguments) {
            Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Err(error) => {
                self.notice = Some(format!("Could not restart as administrator: {error}"));
            }
        }
    }

    fn elevation_hint(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() / 3.0);
            ui.heading("Administrator rights required");
            ui.add_space(8.0);
            ui.label(format!(
                "Indexing {} reads the NTFS master file table, which Windows only allows for administrators.",
                self.scope
            ));
            ui.add_space(12.0);
            if ui.button("Restart as administrator").clicked() {
                self.restart_as_administrator(ui.ctx());
            }
        });
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let focus_search = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::F);
        let copy_path = egui::KeyboardShortcut::new(
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::C,
        );
        let clear = egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::Escape);

        if ctx.input_mut(|input| input.consume_shortcut(&focus_search)) {
            self.focus_search = true;
        }
        if ctx.input_mut(|input| input.consume_shortcut(&copy_path)) {
            self.run_selected_action(ctx, RowAction::CopyPath);
        }
        if ctx.input_mut(|input| input.consume_shortcut(&clear)) {
            self.clear_search();
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let has_selection = self.selected_row().is_some();

        ui.menu_button("File", |ui| {
            if !self.is_elevated && ui.button("Restart as administrator").clicked() {
                self.restart_as_administrator(ui.ctx());
            }
            if ui.button("Exit").clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });

        ui.menu_button("Edit", |ui| {
            if ui
                .add_enabled(has_selection, egui::Button::new("Copy name"))
                .clicked()
            {
                self.run_selected_action(ui.ctx(), RowAction::CopyName);
            }
            if ui
                .add_enabled(
                    has_selection,
                    egui::Button::new("Copy full path").shortcut_text("Ctrl+Shift+C"),
                )
                .clicked()
            {
                self.run_selected_action(ui.ctx(), RowAction::CopyPath);
            }
            ui.separator();
            if ui
                .add(egui::Button::new("Clear search").shortcut_text("Esc"))
                .clicked()
            {
                self.clear_search();
            }
        });

        ui.menu_button("View", |ui| {
            ui.checkbox(&mut self.show_status_bar, "Status bar");
            ui.separator();
            ui.menu_button("Sort by", |ui| {
                let mut sort = self.sort;
                for column in SortColumn::ALL {
                    ui.radio_value(&mut sort.column, column, column.label());
                }
                ui.separator();
                ui.radio_value(&mut sort.descending, false, "Ascending");
                ui.radio_value(&mut sort.descending, true, "Descending");
                self.set_sort(sort);
            });
            ui.menu_button("Theme", |ui| {
                let mut theme = ui.options(|options| options.theme_preference);
                theme.radio_buttons(ui);
                ui.ctx().set_theme(theme);
            });
            ui.separator();
            egui::gui_zoom::zoom_menu_buttons(ui);
        });

        ui.menu_button("Search", |ui| {
            if ui
                .add(egui::Button::new("Focus search field").shortcut_text("Ctrl+F"))
                .clicked()
            {
                self.focus_search = true;
            }
            if ui
                .add_enabled(
                    has_selection,
                    egui::Button::new("Open selected").shortcut_text("Enter"),
                )
                .clicked()
            {
                self.run_selected_action(ui.ctx(), RowAction::Open(OpenAction::Open));
            }
            if ui
                .add_enabled(
                    has_selection,
                    egui::Button::new("Show selected in folder").shortcut_text("Ctrl+Enter"),
                )
                .clicked()
            {
                self.run_selected_action(ui.ctx(), RowAction::Open(OpenAction::Reveal));
            }
        });

        ui.menu_button("Help", |ui| {
            if ui.button("Search syntax").clicked() {
                self.show_syntax_help = true;
            }
            if ui.button(format!("About {APP_NAME}")).clicked() {
                self.show_about = true;
            }
        });
    }

    fn search_bar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        let response = ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .id(egui::Id::new(SEARCH_FIELD_ID))
                .hint_text("Search files   ^prefix   =exact   path:text   sort:name")
                .font(egui::FontId::proportional(16.0))
                .margin(egui::Margin::symmetric(8, 6))
                .desired_width(f32::INFINITY),
        );
        if std::mem::take(&mut self.focus_search) {
            response.request_focus();
        }
        if response.changed() {
            self.send_search();
        }
        // Enter ends single-line editing, which reports the field as having lost focus.
        if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
            let open = if ui.input(|input| input.modifiers.command) {
                OpenAction::Reveal
            } else {
                OpenAction::Open
            };
            let index = self.selected_row().unwrap_or(0);
            self.run_row_action(ui.ctx(), index, RowAction::Open(open));
            self.focus_search = true;
        }
        ui.add_space(4.0);
    }

    fn results_table(&mut self, ui: &mut egui::Ui) {
        let row_height = ui.text_style_height(&egui::TextStyle::Body) + 8.0;
        let mut clicked_column = None;
        let mut selected = self.selected;
        let mut action = None;

        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .sense(egui::Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(300.0).at_least(120.0).clip(true))
            .column(Column::initial(520.0).at_least(160.0).clip(true))
            .column(Column::remainder().at_least(80.0).clip(true))
            .header(row_height + 4.0, |mut header| {
                for column in [SortColumn::Name, SortColumn::Path, SortColumn::Type] {
                    header.col(|ui| {
                        let active = self.sort.column == column;
                        if ui
                            .selectable_label(active, self.sort.header_label(column))
                            .clicked()
                        {
                            clicked_column = Some(column);
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(row_height, self.rows.len(), |mut row| {
                    let index = row.index();
                    let result = &self.rows[index];
                    row.set_selected(selected == Some(index));
                    row.col(|ui| {
                        let icon = if result.is_folder {
                            "\u{1F5C0}"
                        } else {
                            "\u{1F5CB}"
                        };
                        ui.weak(icon);
                        ui.label(&result.name);
                    });
                    row.col(|ui| {
                        ui.label(&result.folder);
                    });
                    row.col(|ui| {
                        ui.weak(&result.kind);
                    });

                    let response = row.response();
                    if response.clicked() || response.secondary_clicked() {
                        selected = Some(index);
                    }
                    if response.double_clicked() {
                        action = Some((index, RowAction::Open(OpenAction::Open)));
                    }
                    response.context_menu(|ui| {
                        let items = [
                            ("Open", RowAction::Open(OpenAction::Open)),
                            ("Show in folder", RowAction::Open(OpenAction::Reveal)),
                            ("Copy name", RowAction::CopyName),
                            ("Copy full path", RowAction::CopyPath),
                        ];
                        for (position, (label, item)) in items.into_iter().enumerate() {
                            if position == 2 {
                                ui.separator();
                            }
                            if ui.button(label).clicked() {
                                action = Some((index, item));
                            }
                        }
                    });
                });
            });

        self.selected = selected;
        if let Some(column) = clicked_column {
            self.set_sort(self.sort.toggled(column));
        }
        if let Some((index, action)) = action {
            self.run_row_action(ui.ctx(), index, action);
        }
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if self.is_indexing {
                ui.spinner();
            } else {
                ui.strong(format!("{} results", self.rows.len()));
                ui.separator();
            }
            ui.label(&self.index_status);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(self.sort.status_label());
                if let Some(notice) = &self.notice {
                    ui.separator();
                    ui.colored_label(ui.visuals().warn_fg_color, notice);
                }
            });
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        egui::Window::new("Search syntax")
            .open(&mut self.show_syntax_help)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                egui::Grid::new("search_syntax")
                    .striped(true)
                    .spacing([16.0, 6.0])
                    .show(ui, |ui| {
                        for (syntax, meaning) in SEARCH_SYNTAX {
                            ui.monospace(syntax);
                            ui.label(meaning);
                            ui.end_row();
                        }
                    });
            });

        egui::Window::new(format!("About {APP_NAME}"))
            .open(&mut self.show_about)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.heading(format!("{APP_NAME} {}", env!("CARGO_PKG_VERSION")));
                ui.label("Fast file-name search for Windows.");
                ui.label(format!("Index scope: {}", self.scope));
            });
    }
}

impl eframe::App for SearchApp {
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_responses();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_shortcuts(&ctx);

        egui::Panel::top("menu_bar").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| self.menu_bar(ui));
        });
        egui::Panel::top("search_bar").show(ui, |ui| self.search_bar(ui));
        if self.show_status_bar {
            egui::Panel::bottom("status_bar").show(ui, |ui| self.status_bar(ui));
        }
        egui::CentralPanel::default().show(ui, |ui| {
            if self.indexing_failed && !self.is_elevated {
                self.elevation_hint(ui);
            } else {
                self.results_table(ui);
            }
        });

        self.dialogs(&ctx);
    }
}

fn spawn_search_worker(
    scope: IndexScope,
    command_tx: mpsc::Sender<WorkerCommand>,
    command_rx: mpsc::Receiver<WorkerCommand>,
    response_tx: ResponseSender,
) {
    thread::spawn(move || match scope {
        #[cfg(windows)]
        IndexScope::LocalDrives => run_search_worker(
            NtfsMultiVolumeSource::new(local_fixed_volumes()),
            command_tx,
            command_rx,
            response_tx,
        ),
        #[cfg(not(windows))]
        IndexScope::LocalDrives => run_search_worker(
            DirectorySource::local_drives(),
            command_tx,
            command_rx,
            response_tx,
        ),
        #[cfg(windows)]
        IndexScope::Root(root) if is_volume_root(&root) => run_search_worker(
            NtfsMftSource::new(root),
            command_tx,
            command_rx,
            response_tx,
        ),
        IndexScope::Root(root) => run_search_worker(
            DirectorySource::new(root),
            command_tx,
            command_rx,
            response_tx,
        ),
    });
}

fn run_search_worker<S: IndexSource + Send + 'static>(
    source: S,
    command_tx: mpsc::Sender<WorkerCommand>,
    command_rx: mpsc::Receiver<WorkerCommand>,
    response_tx: ResponseSender,
) {
    let indexing_started = Instant::now();
    let mut index = match SearchIndex::from_source(&source) {
        Ok(index) => index,
        Err(error) => {
            let _ = response_tx.send(WorkerResponse::IndexingFailed(error.to_string()));
            return;
        }
    };

    if response_tx
        .send(WorkerResponse::IndexReady {
            indexed_items: index.len(),
            indexing_issues: index.indexing_issue_count(),
            indexing_duration: indexing_started.elapsed(),
            initial_results: index.search_results("", RESULT_LIMIT),
        })
        .is_err()
    {
        return;
    }

    spawn_update_poller(source, index.journal_checkpoints().to_vec(), command_tx);

    let mut latest = SearchRequest::default();
    while let Ok(mut command) = command_rx.recv() {
        if let WorkerCommand::Search(_) = command {
            while let Ok(next) = command_rx.try_recv() {
                match next {
                    WorkerCommand::Search(newer_request) => {
                        command = WorkerCommand::Search(newer_request);
                    }
                    other => {
                        if response_tx
                            .send(handle_command(&mut index, &mut latest, command))
                            .is_err()
                        {
                            return;
                        }
                        command = other;
                        break;
                    }
                }
            }
        }
        if response_tx
            .send(handle_command(&mut index, &mut latest, command))
            .is_err()
        {
            return;
        }
    }
}

/// Handles one worker command; the index is only touched here, so every response reflects one snapshot.
fn handle_command(
    index: &mut SearchIndex,
    latest: &mut SearchRequest,
    command: WorkerCommand,
) -> WorkerResponse {
    match command {
        WorkerCommand::Search(request) => {
            let response = SearchResponse {
                revision: request.revision,
                results: index.search_results(&request.query, RESULT_LIMIT),
            };
            *latest = request;
            WorkerResponse::SearchResults(response)
        }
        WorkerCommand::ApplyUpdates(batch) => {
            index.apply(batch);
            WorkerResponse::IndexUpdated {
                indexed_items: index.len(),
                indexing_issues: index.indexing_issue_count(),
                refreshed: SearchResponse {
                    revision: latest.revision,
                    results: index.search_results(&latest.query, RESULT_LIMIT),
                },
            }
        }
        WorkerCommand::UpdateFailed(message) => WorkerResponse::UpdateFailed(message),
    }
}

fn spawn_update_poller<S: IndexSource + Send + 'static>(
    source: S,
    mut checkpoints: Vec<JournalCheckpoint>,
    command_tx: mpsc::Sender<WorkerCommand>,
) {
    thread::spawn(move || {
        loop {
            match poll_updates(&source, &mut checkpoints) {
                ControlFlow::Break(()) => return,
                ControlFlow::Continue(Some(command)) => {
                    if command_tx.send(command).is_err() {
                        return;
                    }
                }
                ControlFlow::Continue(None) => {}
            }
            thread::sleep(UPDATE_POLL_INTERVAL);
        }
    });
}

/// Collects one batch after `checkpoints`, advancing them only when collection succeeds.
///
/// Breaks for sources without incremental updates. Empty batches are not sent; the next
/// non-empty batch carries the advanced checkpoints to the index.
fn poll_updates(
    source: &impl IndexSource,
    checkpoints: &mut Vec<JournalCheckpoint>,
) -> ControlFlow<(), Option<WorkerCommand>> {
    match UpdateBatch::collect(source, checkpoints) {
        Ok(batch) => {
            checkpoints.clone_from(&batch.checkpoints);
            ControlFlow::Continue((!batch.is_empty()).then_some(WorkerCommand::ApplyUpdates(batch)))
        }
        Err(error) if error.kind() == io::ErrorKind::Unsupported => ControlFlow::Break(()),
        Err(error) => ControlFlow::Continue(Some(WorkerCommand::UpdateFailed(format!(
            "Index update failed: {error}"
        )))),
    }
}

fn split_result_path(path_str: &str) -> (String, String) {
    let trimmed = path_str.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return (path_str.to_owned(), String::new());
    }
    if let Some(pos) = trimmed.rfind(['/', '\\']) {
        let name = trimmed[pos + 1..].to_owned();
        let mut folder = trimmed[..pos].to_owned();
        if folder.ends_with(':') {
            folder.push(path_str.chars().nth(pos).unwrap_or('\\'));
        } else if folder.is_empty() && (path_str.starts_with('/') || path_str.starts_with('\\')) {
            folder.push(path_str.chars().next().unwrap_or('/'));
        }
        (name, folder)
    } else {
        (path_str.to_owned(), String::new())
    }
}

/// Returns whether `path` names exactly the root directory of a local drive or volume, such as `C:\` or `/`.
#[cfg_attr(not(windows), allow(dead_code))]
fn is_volume_root(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};

        let mut components = path.components();
        let is_drive = matches!(
            components.next(),
            Some(Component::Prefix(prefix))
                if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        );
        is_drive
            && matches!(components.next(), Some(Component::RootDir))
            && components.next().is_none()
    }
    #[cfg(not(windows))]
    {
        if path == Path::new("/") {
            return true;
        }
        let lossy = path.to_string_lossy();
        let stripped = lossy.strip_prefix(r"\\?\").unwrap_or(&lossy);
        let bytes = stripped.as_bytes();
        if bytes.len() == 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (bytes[2] == b'\\' || bytes[2] == b'/')
        {
            return true;
        }
        false
    }
}

fn describe_attributes(attributes: u32) -> String {
    let kind = if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        "Folder"
    } else {
        "File"
    };
    [
        (FILE_ATTRIBUTE_READONLY, "read-only"),
        (FILE_ATTRIBUTE_HIDDEN, "hidden"),
        (FILE_ATTRIBUTE_SYSTEM, "system"),
    ]
    .into_iter()
    .filter(|(flag, _)| attributes & flag != 0)
    .fold(kind.to_owned(), |description, (_, label)| {
        format!("{description}, {label}")
    })
}

fn format_indexing_status(scope: impl fmt::Display) -> String {
    format!("Indexing {scope}...")
}

fn format_index_status(
    scope: impl fmt::Display,
    indexing_duration: Duration,
    indexed_items: usize,
    indexing_issues: usize,
) -> String {
    let status = format!(
        "Indexed {indexed_items} items from {scope} in {} ms",
        indexing_duration.as_millis()
    );

    if indexing_issues == 0 {
        return status;
    }

    let path_label = if indexing_issues == 1 {
        "path"
    } else {
        "paths"
    };
    format!("{status} ({indexing_issues} inaccessible {path_label})")
}

fn format_update_status(indexed_items: usize, indexing_issues: usize) -> String {
    let status = format!("Index updated: {indexed_items} items");
    match indexing_issues {
        0 => status,
        1 => format!("{status} (1 issue)"),
        _ => format!("{status} ({indexing_issues} issues)"),
    }
}

#[cfg(test)]
mod tests {
    use std::{io, ops::ControlFlow, path::Path, time::Duration};

    use crate::search::{
        EntryMetadata, IndexIssue, IndexSource, IndexUpdate, JournalCheckpoint, SearchEntry,
        SearchIndex, SearchResult, UpdateBatch, WorkspaceSource,
    };

    use crate::platform::{OpenAction, explorer_argument};

    use super::{
        IndexScope, ResultRow, ResultSort, RowAction, SearchApp, SearchRequest, SearchResponse,
        SortColumn, WorkerCommand, WorkerResponse, describe_attributes, effective_query,
        format_index_status, format_indexing_status, format_update_status, handle_command,
        is_volume_root, poll_updates,
    };

    fn displays(results: &[SearchResult]) -> Vec<&str> {
        results
            .iter()
            .map(|result| result.display.as_str())
            .collect()
    }

    #[test]
    fn describes_files_and_folders_with_their_attributes() {
        assert_eq!(describe_attributes(0), "File");
        assert_eq!(describe_attributes(0x0000_0010), "Folder");
        assert_eq!(
            describe_attributes(0x0000_0001 | 0x0000_0002 | 0x0000_0004 | 0x0000_0020),
            "File, read-only, hidden, system",
        );
    }

    #[test]
    fn splits_result_paths_into_name_and_folder() {
        let row = ResultRow::from_result(SearchResult {
            display: r"nested\a.txt".to_owned(),
            path: r"C:\root\nested\a.txt".to_owned(),
            path_resolved: true,
            attributes: 0x0000_0001,
        });

        assert_eq!(
            row,
            ResultRow {
                name: "a.txt".to_owned(),
                folder: r"C:\root\nested".to_owned(),
                kind: "File, read-only".to_owned(),
                is_folder: false,
                path: r"C:\root\nested\a.txt".to_owned(),
                path_resolved: true,
            }
        );
    }

    #[test]
    fn shows_volume_roots_by_their_full_path() {
        let row = ResultRow::from_result(SearchResult {
            display: r"C:\".to_owned(),
            path: r"C:\".to_owned(),
            path_resolved: true,
            attributes: 0x0000_0010,
        });

        assert_eq!(row.name, r"C:\");
        assert_eq!(row.folder, "");
        assert!(row.is_folder);
    }

    #[test]
    fn header_clicks_start_ascending_and_reverse_the_active_column() {
        let by_name = ResultSort::default().toggled(SortColumn::Name);
        assert_eq!(
            by_name,
            ResultSort {
                column: SortColumn::Name,
                descending: false
            }
        );
        assert!(by_name.toggled(SortColumn::Name).descending);
        assert!(
            !by_name
                .toggled(SortColumn::Name)
                .toggled(SortColumn::Name)
                .descending
        );
        assert_eq!(
            by_name.toggled(SortColumn::Name).toggled(SortColumn::Path),
            ResultSort {
                column: SortColumn::Path,
                descending: false
            }
        );
    }

    #[test]
    fn prefixes_non_default_sort_orders_so_typed_sort_terms_win() {
        assert_eq!(effective_query("abc", ResultSort::default()), "abc");
        assert_eq!(
            effective_query(
                "abc sort:type",
                ResultSort {
                    column: SortColumn::Path,
                    descending: true
                }
            ),
            "sort:-path abc sort:type"
        );
    }

    fn result(path: &str) -> SearchResult {
        SearchResult {
            display: path.to_owned(),
            path: path.to_owned(),
            path_resolved: true,
            attributes: 0,
        }
    }

    fn search_results(revision: u64, paths: &[&str]) -> WorkerResponse {
        WorkerResponse::SearchResults(SearchResponse {
            revision,
            results: paths.iter().map(|path| result(path)).collect(),
        })
    }

    #[test]
    fn ignores_results_for_superseded_queries() {
        let mut app = SearchApp {
            latest_revision: 2,
            ..SearchApp::default()
        };

        app.apply_response(search_results(1, &[r"C:\stale.txt"]));
        assert!(app.rows.is_empty());

        app.apply_response(search_results(2, &[r"C:\fresh.txt"]));
        assert_eq!(app.rows.len(), 1);
        assert_eq!(app.rows[0].name, "fresh.txt");
    }

    #[test]
    fn keeps_the_selected_path_selected_across_result_updates() {
        let mut app = SearchApp::default();
        app.apply_response(search_results(0, &[r"C:\a.txt", r"C:\b.txt"]));
        app.selected = Some(1);

        app.apply_response(search_results(0, &[r"C:\b.txt", r"C:\c.txt"]));
        assert_eq!(app.selected, Some(0));

        app.apply_response(search_results(0, &[r"C:\c.txt"]));
        assert_eq!(app.selected, None);
    }

    #[test]
    fn keeps_the_indexing_failure_when_the_worker_disconnects() {
        let mut app = SearchApp {
            is_indexing: true,
            worker_stopped: false,
            ..SearchApp::default()
        };

        app.apply_response(WorkerResponse::IndexingFailed("access denied".to_owned()));
        app.drain_responses();

        assert!(!app.is_indexing);
        assert_eq!(app.index_status, "Indexing failed: access denied");
    }

    #[test]
    fn quotes_paths_for_explorer_and_selects_them_when_revealing() {
        let path = Path::new(r"C:\Program Files\a,b.txt");

        assert_eq!(
            explorer_argument(path, OpenAction::Open),
            r#""C:\Program Files\a,b.txt""#,
        );
        assert_eq!(
            explorer_argument(path, OpenAction::Reveal),
            r#"/select,"C:\Program Files\a,b.txt""#,
        );
    }

    #[derive(Default)]
    struct ChangingSource {
        updates: Vec<IndexUpdate>,
        fails: bool,
    }

    impl IndexSource for ChangingSource {
        fn collect_initial(
            &self,
            out: &mut Vec<SearchEntry>,
            _issues: &mut Vec<IndexIssue>,
        ) -> io::Result<()> {
            out.push(entry(1, None, r"C:\"));
            out.push(entry(2, Some(1), "alpha.txt"));
            Ok(())
        }

        fn collect_updates(
            &self,
            checkpoints: &mut [JournalCheckpoint],
            out: &mut Vec<IndexUpdate>,
            _issues: &mut Vec<IndexIssue>,
        ) -> io::Result<()> {
            if self.fails {
                return Err(io::Error::other("journal offline"));
            }
            out.extend(self.updates.clone());
            for checkpoint in checkpoints {
                checkpoint.next_usn += 1;
            }
            Ok(())
        }

        fn journal_checkpoints(&self) -> io::Result<Vec<JournalCheckpoint>> {
            Ok(vec![JournalCheckpoint {
                volume_identity: r"C:\".to_owned(),
                journal_id: 1,
                next_usn: 10,
            }])
        }
    }

    fn entry(file_id: u128, parent_id: Option<u128>, name: &str) -> SearchEntry {
        SearchEntry {
            display: name.to_owned(),
            path: name.to_owned(),
            path_resolved: false,
            metadata: EntryMetadata {
                file_id: Some(file_id),
                parent_id,
                name: name.to_owned(),
                full_path: name.into(),
                volume_identity: Some(r"C:\".to_owned()),
                attributes: 0,
            },
        }
    }

    #[test]
    fn rejects_unresolved_index_entries_when_preparing_opening_paths() {
        for (parent_id, path) in [
            (99, "orphan.txt"),
            (3, "orphan.txt"),
            (99, r"C:\orphan.txt"),
        ] {
            let mut orphan = entry(3, Some(parent_id), "orphan.txt");
            orphan.path = path.to_owned();
            orphan.metadata.full_path = path.into();
            let source = ChangingSource {
                updates: vec![IndexUpdate::Upsert(orphan)],
                ..ChangingSource::default()
            };
            let mut index = SearchIndex::from_source(&source).unwrap();
            index.refresh(&source).unwrap();
            let result = index.search_results("=orphan.txt", 1).pop().unwrap();
            let row = ResultRow::from_result(result);

            assert_eq!(row.path, path);
            assert_eq!(row.name, "orphan.txt");
            assert_eq!(
                row.action_path().unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );

            let context = eframe::egui::Context::default();
            let mut app = SearchApp {
                rows: vec![row],
                ..SearchApp::default()
            };
            for action in [OpenAction::Open, OpenAction::Reveal] {
                app.run_row_action(&context, 0, RowAction::Open(action));
                assert!(app.notice.as_deref().is_some_and(|notice| {
                    notice.contains("the indexed path could not be resolved")
                }));
                app.notice = None;
            }
        }
    }

    #[test]
    fn rejects_previously_resolved_entries_when_preparing_opening_paths_after_parent_removal() {
        let source = ChangingSource {
            updates: vec![IndexUpdate::Remove {
                volume_identity: r"C:\".to_owned(),
                file_id: 1,
            }],
            ..ChangingSource::default()
        };
        let mut index = SearchIndex::from_source(&source).unwrap();
        assert!(index.search_results("=alpha.txt", 1)[0].path_resolved);
        index.refresh(&source).unwrap();
        let result = index.search_results("=alpha.txt", 1).pop().unwrap();
        let row = ResultRow::from_result(result);

        assert_eq!(row.path, r"C:\alpha.txt");
        assert_eq!(
            row.action_path().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn preserves_relative_workspace_entries_when_preparing_opening_paths() {
        let path = Path::new("nested/workspace-relative.txt");
        let source = ChangingSource {
            updates: vec![IndexUpdate::Upsert(SearchEntry::from_path(
                "workspace-relative.txt".to_owned(),
                path.to_path_buf(),
            ))],
            ..ChangingSource::default()
        };
        let mut index = SearchIndex::from_source(&source).unwrap();
        index.refresh(&source).unwrap();
        let result = index
            .search_results("=workspace-relative.txt", 1)
            .pop()
            .unwrap();
        let row = ResultRow::from_result(result);

        assert_eq!(
            row.action_path().unwrap(),
            std::path::absolute(path).unwrap()
        );
    }

    #[test]
    fn answers_searches_and_remembers_the_latest_query() {
        let mut index = SearchIndex::from_source(&ChangingSource::default()).unwrap();
        let mut latest = SearchRequest::default();

        let response = handle_command(
            &mut index,
            &mut latest,
            WorkerCommand::Search(SearchRequest {
                revision: 3,
                query: "alp".to_owned(),
            }),
        );

        let WorkerResponse::SearchResults(response) = response else {
            panic!("a search must be answered with search results");
        };
        assert_eq!(response.revision, 3);
        assert_eq!(displays(&response.results), vec![r"C:\alpha.txt"]);
        assert_eq!(latest.query, "alp");
    }

    #[test]
    fn applies_update_batches_and_refreshes_the_latest_query() {
        let source = ChangingSource {
            updates: vec![IndexUpdate::Upsert(entry(3, Some(1), "alpine.txt"))],
            ..ChangingSource::default()
        };
        let mut index = SearchIndex::from_source(&source).unwrap();
        let batch = UpdateBatch::collect(&source, index.journal_checkpoints()).unwrap();
        let mut latest = SearchRequest {
            revision: 5,
            query: "alp".to_owned(),
        };

        let response = handle_command(&mut index, &mut latest, WorkerCommand::ApplyUpdates(batch));

        let WorkerResponse::IndexUpdated {
            indexed_items,
            indexing_issues,
            refreshed,
        } = response
        else {
            panic!("an applied batch must publish the updated index");
        };
        assert_eq!(indexed_items, 3);
        assert_eq!(indexing_issues, 0);
        assert_eq!(refreshed.revision, 5);
        assert_eq!(
            displays(&refreshed.results),
            vec![r"C:\alpha.txt", r"C:\alpine.txt"]
        );
    }

    #[test]
    fn forwards_update_failures_to_the_ui() {
        let mut index = SearchIndex::from_source(&ChangingSource::default()).unwrap();

        let response = handle_command(
            &mut index,
            &mut SearchRequest::default(),
            WorkerCommand::UpdateFailed("journal offline".to_owned()),
        );

        assert!(
            matches!(response, WorkerResponse::UpdateFailed(message) if message == "journal offline")
        );
    }

    #[test]
    fn stops_polling_sources_without_incremental_updates() {
        let flow = poll_updates(&WorkspaceSource::new("."), &mut Vec::new());

        assert!(flow.is_break());
    }

    #[test]
    fn emits_collected_batches_and_advances_the_polled_checkpoints() {
        let source = ChangingSource {
            updates: vec![IndexUpdate::Upsert(entry(3, Some(1), "alpine.txt"))],
            ..ChangingSource::default()
        };
        let mut checkpoints = source.journal_checkpoints().unwrap();

        let flow = poll_updates(&source, &mut checkpoints);

        assert!(matches!(
            flow,
            ControlFlow::Continue(Some(WorkerCommand::ApplyUpdates(batch))) if batch.updates.len() == 1
        ));
        assert_eq!(checkpoints[0].next_usn, 11);
    }

    #[test]
    fn skips_empty_batches_but_keeps_their_checkpoints() {
        let source = ChangingSource::default();
        let mut checkpoints = source.journal_checkpoints().unwrap();

        let flow = poll_updates(&source, &mut checkpoints);

        assert!(matches!(flow, ControlFlow::Continue(None)));
        assert_eq!(checkpoints[0].next_usn, 11);
    }

    #[test]
    fn reports_update_failures_without_advancing_the_polled_checkpoints() {
        let source = ChangingSource {
            fails: true,
            ..ChangingSource::default()
        };
        let mut checkpoints = source.journal_checkpoints().unwrap();

        let flow = poll_updates(&source, &mut checkpoints);

        assert!(matches!(
            flow,
            ControlFlow::Continue(Some(WorkerCommand::UpdateFailed(message))) if message.contains("journal offline")
        ));
        assert_eq!(checkpoints[0].next_usn, 10);
    }

    #[test]
    fn treats_only_drive_roots_as_volume_selections() {
        assert!(is_volume_root(Path::new(r"C:\")));
        assert!(is_volume_root(Path::new(r"\\?\D:\")));
        assert!(!is_volume_root(Path::new("C:")));
        assert!(!is_volume_root(Path::new(r"C:\Users")));
        assert!(!is_volume_root(Path::new("relative")));
        assert!(!is_volume_root(Path::new(r"\\server\share\")));
    }

    #[test]
    fn reports_item_and_issue_counts_after_an_index_update() {
        assert_eq!(format_update_status(3, 0), "Index updated: 3 items");
        assert_eq!(
            format_update_status(3, 1),
            "Index updated: 3 items (1 issue)"
        );
        assert_eq!(
            format_update_status(3, 2),
            "Index updated: 3 items (2 issues)"
        );
    }

    #[test]
    fn reports_the_index_root_while_initial_indexing_is_in_progress() {
        assert_eq!(
            format_indexing_status(Path::new(r"C:\search-root").display()),
            r"Indexing C:\search-root...",
        );
    }

    #[test]
    fn reports_index_root_and_duration_in_the_startup_status() {
        assert_eq!(
            format_index_status(
                Path::new(r"C:\search-root").display(),
                Duration::from_millis(250),
                12,
                0
            ),
            r"Indexed 12 items from C:\search-root in 250 ms",
        );
    }

    #[test]
    fn omits_issue_information_when_indexing_completes_without_issues() {
        assert_eq!(
            format_index_status(
                Path::new(r"C:\search-root").display(),
                Duration::ZERO,
                12,
                0
            ),
            r"Indexed 12 items from C:\search-root in 0 ms",
        );
    }

    #[test]
    fn reports_recoverable_indexing_issues_in_the_startup_status() {
        assert_eq!(
            format_index_status(
                Path::new(r"C:\search-root").display(),
                Duration::ZERO,
                12,
                2
            ),
            r"Indexed 12 items from C:\search-root in 0 ms (2 inaccessible paths)",
        );
    }

    #[test]
    fn uses_the_singular_wording_for_one_indexing_issue() {
        assert_eq!(
            format_index_status(Path::new(r"C:\search-root").display(), Duration::ZERO, 1, 1),
            r"Indexed 1 items from C:\search-root in 0 ms (1 inaccessible path)",
        );
    }

    #[test]
    fn describes_the_local_drive_scope_in_the_status() {
        assert_eq!(
            format_indexing_status(IndexScope::LocalDrives),
            "Indexing all local drives...",
        );
        assert_eq!(
            format_indexing_status(IndexScope::Root(r"D:\".into())),
            r"Indexing D:\...",
        );
    }
}
