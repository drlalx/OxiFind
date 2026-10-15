use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Cell, Clear, HighlightSpacing, Paragraph, Row, Table,
    },
};

use crate::engine::{describe_attributes, split_result_path, SortColumn};
use super::app::{FocusedWidget, TuiApp, TuiModal};

// UI Palette matching OxiDisc modern dark theme
pub const COLOR_BG: Color = Color::Rgb(22, 29, 41);
pub const COLOR_CARD: Color = Color::Rgb(26, 35, 50);
pub const COLOR_BORDER: Color = Color::Rgb(42, 54, 72);
pub const COLOR_BORDER_FOCUS: Color = Color::Rgb(0, 230, 210);
pub const COLOR_ACCENT: Color = Color::Rgb(0, 230, 210);
pub const COLOR_TEXT: Color = Color::Rgb(230, 238, 248);
pub const COLOR_MUTED: Color = Color::Rgb(140, 155, 175);
pub const COLOR_DIM: Color = Color::Rgb(90, 105, 125);
pub const COLOR_HIGHLIGHT: Color = Color::Rgb(250, 204, 21);
pub const COLOR_SUCCESS: Color = Color::Rgb(34, 197, 94);
pub const COLOR_DANGER: Color = Color::Rgb(239, 68, 68);

pub fn render(frame: &mut Frame, app: &mut TuiApp) {
    let size = frame.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header & status bar
            Constraint::Length(3), // Search Input & chips
            Constraint::Min(5),    // Result Table
            Constraint::Length(3), // Selected Item Details
            Constraint::Length(1), // Footer Keybinding hints
        ])
        .split(size);

    render_header(frame, app, chunks[0]);
    render_search_bar(frame, app, chunks[1]);
    render_table(frame, app, chunks[2]);
    render_details(frame, app, chunks[3]);
    render_footer(frame, app, chunks[4]);

    if let Some(modal) = &app.active_modal {
        render_modal(frame, modal, size);
    }
}

fn render_header(frame: &mut Frame, app: &TuiApp, area: Rect) {
    let index_status_span = if app.is_indexing {
        Span::styled(
            " [INDEXING...] ",
            Style::default().fg(COLOR_HIGHLIGHT).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            format!(" [INDEXED: {} items] ", app.indexed_items),
            Style::default().fg(COLOR_SUCCESS).add_modifier(Modifier::BOLD),
        )
    };

    let scope_span = Span::styled(
        format!(" Scope: {} ", app.scope),
        Style::default().fg(COLOR_ACCENT).bg(COLOR_CARD),
    );

    let header_line = Line::from(vec![
        Span::styled(
            " OxiFind ",
            Style::default()
                .fg(Color::Black)
                .bg(COLOR_ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        index_status_span,
        Span::raw(" "),
        scope_span,
    ]);

    let sort_label = app.sort.label();
    let sort_span = Span::styled(
        format!(" {sort_label} "),
        Style::default().fg(COLOR_MUTED),
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(COLOR_BORDER))
        .border_type(BorderType::Rounded)
        .title(header_line)
        .title_alignment(Alignment::Left)
        .title(Line::from(vec![sort_span]))
        .title_alignment(Alignment::Right);

    let mut status_spans = Vec::new();
    if let Some((msg, _)) = &app.status_message {
        status_spans.push(Span::styled(
            format!("Notice: {msg}"),
            Style::default().fg(COLOR_HIGHLIGHT).add_modifier(Modifier::BOLD),
        ));
    } else if app.indexing_issues > 0 {
        status_spans.push(Span::styled(
            format!("Indexed with {} warnings", app.indexing_issues),
            Style::default().fg(COLOR_MUTED),
        ));
    } else {
        status_spans.push(Span::styled(
            "Ready - Type to search instantly",
            Style::default().fg(COLOR_MUTED),
        ));
    }

    let status_paragraph = Paragraph::new(Line::from(status_spans)).block(block);
    frame.render_widget(status_paragraph, area);
}

fn render_search_bar(frame: &mut Frame, app: &TuiApp, area: Rect) {
    let is_focused = app.focused == FocusedWidget::SearchInput;
    let border_color = if is_focused {
        COLOR_BORDER_FOCUS
    } else {
        COLOR_BORDER
    };

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(20), Constraint::Length(34)])
        .split(area);

    // Left: Search input
    let search_title = if is_focused {
        Line::from(vec![Span::styled(
            " Search (Focused) ",
            Style::default().fg(COLOR_ACCENT).add_modifier(Modifier::BOLD),
        )])
    } else {
        Line::from(vec![Span::styled(
            " Search (<Tab> to focus) ",
            Style::default().fg(COLOR_MUTED),
        )])
    };

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .border_type(BorderType::Rounded)
        .title(search_title);

    let input_text = if app.query.is_empty() && !is_focused {
        Span::styled("Type to search...", Style::default().fg(COLOR_DIM))
    } else {
        Span::styled(&app.query, Style::default().fg(COLOR_TEXT))
    };

    let search_paragraph = Paragraph::new(Line::from(vec![input_text])).block(input_block);
    frame.render_widget(search_paragraph, chunks[0]);

    if is_focused {
        frame.set_cursor_position((
            chunks[0].x + 1 + (app.cursor as u16).min(chunks[0].width.saturating_sub(3)),
            chunks[0].y + 1,
        ));
    }

    // Right: Mode chips
    let chip = |active: bool, key: &str, label: &str| -> Span<'static> {
        if active {
            Span::styled(
                format!(" [{key}:{label}] "),
                Style::default().fg(COLOR_ACCENT).add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(
                format!(" [{key}:{label}] "),
                Style::default().fg(COLOR_DIM),
            )
        }
    };

    let chips_line = Line::from(vec![
        chip(app.options.match_case, "M-c", "Case"),
        chip(app.options.whole_word, "M-w", "Word"),
        chip(app.options.match_path, "M-p", "Path"),
        chip(app.options.regex, "M-r", "Regex"),
    ]);

    let chips_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(COLOR_BORDER))
        .border_type(BorderType::Rounded)
        .title(" Toggles (Alt+Key) ");

    let chips_paragraph = Paragraph::new(chips_line).block(chips_block);
    frame.render_widget(chips_paragraph, chunks[1]);
}

fn render_table(frame: &mut Frame, app: &mut TuiApp, area: Rect) {
    let is_focused = app.focused == FocusedWidget::ResultTable;
    let border_color = if is_focused {
        COLOR_BORDER_FOCUS
    } else {
        COLOR_BORDER
    };

    let title_line = Line::from(vec![
        Span::styled(
            format!(" Results ({} matches) ", app.results.len()),
            Style::default().fg(if is_focused { COLOR_ACCENT } else { COLOR_TEXT }),
        ),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .border_type(BorderType::Rounded)
        .title(title_line);

    let header_cells = [
        SortColumn::Name,
        SortColumn::Path,
        SortColumn::Type,
    ]
    .iter()
    .map(|col| {
        let label = app.sort.header_label(*col);
        Cell::from(label).style(
            Style::default()
                .fg(if app.sort.column == *col { COLOR_ACCENT } else { COLOR_TEXT })
                .add_modifier(Modifier::BOLD),
        )
    });

    let header_row = Row::new(header_cells)
        .style(Style::default().bg(COLOR_CARD))
        .height(1)
        .bottom_margin(1);

    let query_lower = app.query.to_lowercase();

    let rows: Vec<Row> = app
        .results
        .iter()
        .map(|res| {
            let (name, folder) = split_result_path(&res.path);
            let kind = describe_attributes(res.attributes);

            // Highlight matches in filename
            let name_span = if !query_lower.is_empty() && name.to_lowercase().contains(&query_lower) {
                let name_lower = name.to_lowercase();
                if let Some(pos) = name_lower.find(&query_lower) {
                    let end = pos + query_lower.len();
                    Line::from(vec![
                        Span::raw(name[..pos].to_owned()),
                        Span::styled(
                            name[pos..end].to_owned(),
                            Style::default()
                                .fg(COLOR_HIGHLIGHT)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(name[end..].to_owned()),
                    ])
                } else {
                    Line::from(name)
                }
            } else {
                Line::from(name)
            };

            Row::new(vec![
                Cell::from(name_span),
                Cell::from(Span::styled(folder, Style::default().fg(COLOR_MUTED))),
                Cell::from(Span::styled(kind, Style::default().fg(COLOR_DIM))),
            ])
        })
        .collect();

    let widths = [
        Constraint::Percentage(35),
        Constraint::Percentage(50),
        Constraint::Percentage(15),
    ];

    let table = Table::new(rows, widths)
        .header(header_row)
        .block(block)
        .highlight_spacing(HighlightSpacing::Always)
        .highlight_symbol("> ")
        .row_highlight_style(
            Style::default()
                .bg(COLOR_CARD)
                .fg(COLOR_ACCENT)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_stateful_widget(table, area, &mut app.table_state);
}

fn render_details(frame: &mut Frame, app: &TuiApp, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(COLOR_BORDER))
        .border_type(BorderType::Rounded)
        .title(" Selected Item ");

    let content = if let Some(res) = app.selected_result() {
        let kind = describe_attributes(res.attributes);
        Line::from(vec![
            Span::styled("Path: ", Style::default().fg(COLOR_MUTED)),
            Span::styled(&res.path, Style::default().fg(COLOR_TEXT).add_modifier(Modifier::BOLD)),
            Span::raw("  |  "),
            Span::styled(format!("Type: {kind}"), Style::default().fg(COLOR_DIM)),
        ])
    } else {
        Line::from(vec![Span::styled(
            "No file selected",
            Style::default().fg(COLOR_DIM),
        )])
    };

    let paragraph = Paragraph::new(content).block(block);
    frame.render_widget(paragraph, area);
}

fn render_footer(frame: &mut Frame, _app: &TuiApp, area: Rect) {
    let keys = Line::from(vec![
        Span::styled("<Enter>", Style::default().fg(COLOR_ACCENT)),
        Span::raw(" Open  "),
        Span::styled("<Ctrl+O>", Style::default().fg(COLOR_ACCENT)),
        Span::raw(" Reveal  "),
        Span::styled("<Tab>", Style::default().fg(COLOR_ACCENT)),
        Span::raw(" Switch Pane  "),
        Span::styled("<F1/?>", Style::default().fg(COLOR_ACCENT)),
        Span::raw(" Help  "),
        Span::styled("<F2>", Style::default().fg(COLOR_ACCENT)),
        Span::raw(" Scope  "),
        Span::styled("<Alt+1..4>", Style::default().fg(COLOR_ACCENT)),
        Span::raw(" Sort  "),
        Span::styled("<Ctrl+Q>", Style::default().fg(COLOR_ACCENT)),
        Span::raw(" Quit"),
    ]);

    let paragraph = Paragraph::new(keys).alignment(Alignment::Center);
    frame.render_widget(paragraph, area);
}

fn render_modal(frame: &mut Frame, modal: &TuiModal, area: Rect) {
    let popup_area = centered_rect(65, 65, area);
    frame.render_widget(Clear, popup_area);

    match modal {
        TuiModal::Help => {
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(COLOR_BORDER_FOCUS))
                .border_type(BorderType::Double)
                .title(" Keyboard Shortcuts & Help (<Esc> to close) ")
                .title_alignment(Alignment::Center);

            let help_text = vec![
                Line::from(Span::styled("Navigation & Selection", Style::default().fg(COLOR_ACCENT).add_modifier(Modifier::BOLD))),
                Line::from("  ↓ / j, ↑ / k       Navigate through results list"),
                Line::from("  PageDown / PageUp  Fast page navigation"),
                Line::from("  Home / End         Jump to first or last result"),
                Line::from("  Tab                Toggle focus between Search Input and Results Table"),
                Line::from(""),
                Line::from(Span::styled("Actions", Style::default().fg(COLOR_ACCENT).add_modifier(Modifier::BOLD))),
                Line::from("  Enter              Open file with system default handler"),
                Line::from("  Ctrl+O / Alt+O     Reveal item in native file manager"),
                Line::from("  Ctrl+R / F5        Force re-index / refresh"),
                Line::from("  Esc                Clear search query / Close active dialog"),
                Line::from("  Ctrl+Q / Ctrl+C    Exit OxiFind"),
                Line::from(""),
                Line::from(Span::styled("Search & Filters", Style::default().fg(COLOR_ACCENT).add_modifier(Modifier::BOLD))),
                Line::from("  Alt+c              Toggle Match Case"),
                Line::from("  Alt+w              Toggle Whole Word"),
                Line::from("  Alt+p              Toggle Match Path"),
                Line::from("  Alt+r              Toggle Regular Expression"),
                Line::from("  Alt+1 .. Alt+4     Sort by Relevance, Name, Path, Type"),
            ];

            let paragraph = Paragraph::new(help_text).block(block);
            frame.render_widget(paragraph, popup_area);
        }
        TuiModal::ScopePicker { scopes, selected } => {
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(COLOR_BORDER_FOCUS))
                .border_type(BorderType::Double)
                .title(" Select Search Scope (<Enter> to select, <Esc> to cancel) ")
                .title_alignment(Alignment::Center);

            let mut lines = Vec::new();
            lines.push(Line::from(Span::styled(
                "Choose drive or directory to index:",
                Style::default().fg(COLOR_MUTED),
            )));
            lines.push(Line::from(""));

            for (i, scope) in scopes.iter().enumerate() {
                let is_selected = i == *selected;
                let symbol = if is_selected { "> " } else { "  " };
                let style = if is_selected {
                    Style::default().fg(COLOR_ACCENT).bg(COLOR_CARD).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(COLOR_TEXT)
                };
                lines.push(Line::from(vec![
                    Span::styled(symbol, style),
                    Span::styled(format!("{scope}"), style),
                ]));
            }

            let paragraph = Paragraph::new(lines).block(block);
            frame.render_widget(paragraph, popup_area);
        }
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
