use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table, Wrap},
};

use crate::app::{App, Focus, RunProfile, TargetStatusKind};

const INACTIVE_COLOR: Color = Color::Indexed(0);
const ACTIVE_COLOR: Color = Color::Indexed(201);
const ACTIVE_TEXT_COLOR: Color = Color::Indexed(7);
const INACTIVE_TEXT_COLOR: Color = Color::Indexed(8);
const SELECTED_TEXT_COLOR: Color = Color::Indexed(0);

pub fn render(frame: &mut Frame, app: &mut App) {
    let columns = Layout::horizontal([
        Constraint::Percentage(25),
        Constraint::Percentage(37),
        Constraint::Percentage(38),
    ])
    .spacing(1);

    let [left, middle, right] = frame.area().layout(&columns);

    render_metadata(frame, app, left);

    let show_project_search = !app.project_query.is_empty() || (app.focus == Focus::Projects && app.filter_mode);
    let middle_chunks = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(if show_project_search { 3 } else { 0 }),
    ])
    .split(middle);
    render_projects(frame, app, middle_chunks[0]);
    if show_project_search {
        render_search_input(frame, middle_chunks[1], &app.project_input, app.filter_mode, "Search projects");
    }

    let show_target_search = !app.target_query.is_empty() || (app.focus == Focus::Targets && app.filter_mode);
    let has_running = app.running_target_statuses().next().is_some();
    let right_chunks = Layout::vertical([
        Constraint::Length(if has_running { 6 } else { 0 }),
        Constraint::Min(0),
        Constraint::Length(if show_target_search { 3 } else { 0 }),
    ])
    .split(right);
    if has_running {
        render_running(frame, app, right_chunks[0]);
    }
    render_targets(frame, app, right_chunks[1]);
    if show_target_search {
        render_search_input(frame, right_chunks[2], &app.target_input, app.filter_mode, "Search targets");
    }
}

fn render_metadata(frame: &mut Frame, app: &mut App, area: Rect) {
    let (title, body) = if let Some(metadata) = app.project_metadata() {
        let status_style = match metadata.git_status.as_str() {
            "DIRTY" => Style::default().fg(Color::Red),
            _ => Style::default(),
        };

        (
            metadata.package_name.clone(),
            vec![
                Line::from(vec![
                    Span::raw(format!("@{}  {} ", metadata.package_version, metadata.git_branch)),
                    Span::styled(metadata.git_status.clone(), status_style),
                ]),
                Line::default(),
                Line::from(metadata.description.clone()),
            ],
        )
    } else {
        (
            "Metadata".to_string(),
            vec![Line::from("No project selected")],
        )
    };

    frame.render_widget(
        Paragraph::new(body)
            .wrap(Wrap { trim: false })
            .block(
                Block::bordered()
                    .title(Line::from(title).style(Style::default().add_modifier(Modifier::BOLD)))
                    .title_alignment(Alignment::Left)
                    .border_type(BorderType::Rounded),
            )
            .fg(ACTIVE_TEXT_COLOR),
        area,
    );
}

fn render_projects(frame: &mut Frame, app: &mut App, area: Rect) {
    let inner = area.inner(Margin {
        vertical: 1,
        horizontal: 1,
    });
    let visible_rows = inner.height.saturating_sub(1) as usize;
    let scroll_offset = project_scroll_offset(app, visible_rows);

    if app.filtered_projects.is_empty() {
        frame.render_widget(
            Paragraph::new("No matching projects")
                .block(
                    Block::bordered()
                        .title("Projects")
                        .title_alignment(Alignment::Center)
                        .border_type(BorderType::Rounded),
                )
                .fg(if app.focus == Focus::Projects { ACTIVE_COLOR } else { INACTIVE_COLOR }),
            area,
        );
    } else {
        let visible_projects: Vec<_> = app
            .visible_projects()
            .enumerate()
            .skip(scroll_offset)
            .take(visible_rows)
            .map(|(index, project)| (index, project.clone()))
            .collect();

        let rows: Vec<Row> = visible_projects
            .into_iter()
            .map(|(index, project)| {
                let is_selected = app.cursor == index as isize;
                let is_dirty_or_untracked = app
                    .git_status_cache
                    .get(&project.path)
                    .is_some_and(|status| matches!(status.as_str(), "DIRTY" | "UNTRACKED"));
                let base_style = if is_selected {
                    Style::default()
                        .fg(SELECTED_TEXT_COLOR)
                        .bg(ACTIVE_COLOR)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(if app.focus == Focus::Projects {
                        ACTIVE_TEXT_COLOR
                    } else {
                        INACTIVE_TEXT_COLOR
                    })
                };
                let name_style = if is_selected {
                    base_style
                } else if is_dirty_or_untracked {
                    base_style.fg(Color::Red)
                } else {
                    base_style.fg(ACTIVE_TEXT_COLOR)
                };
                let cached_size = app.cached_project_size_bytes(&project);
                let size_text = cached_size.map(format_size).unwrap_or_else(|| "…".to_string());
                let size_style = if is_selected {
                    base_style
                } else if cached_size.is_some_and(|size| size >= 5 * 1024 * 1024 * 1024) {
                    base_style.fg(Color::Indexed(9))
                } else if cached_size.is_some_and(|size| size >= 1024 * 1024 * 1024) {
                    base_style.fg(Color::Indexed(1))
                } else {
                    base_style
                };

                let last_opened = app.project_last_opened(&project);
                let bookmark = if app.is_bookmarked(&project) { "🌟" } else { "" };
                Row::new(vec![
                    Cell::from(bookmark),
                    Cell::from(project.name).style(name_style),
                    Cell::from(size_text).style(size_style),
                    Cell::from(last_opened),
                ])
                .style(base_style)
            })
            .collect();

        frame.render_widget(
            Table::new(rows, [Constraint::Length(2), Constraint::Fill(1), Constraint::Length(10), Constraint::Length(12)])
                .header(Row::new(vec!["", "Name", "Size", "Last Opened"]).style(Style::default().add_modifier(Modifier::BOLD)))
                .block(
                    Block::bordered()
                        .title("Projects")
                        .title_alignment(Alignment::Center)
                        .border_type(BorderType::Rounded),
                )
                .fg(if app.focus == Focus::Projects { ACTIVE_COLOR } else { INACTIVE_COLOR }),
            area,
        );
    }

    if !app.filtered_projects.is_empty() {
        let mut scrollbar_state = ScrollbarState::new(app.filtered_projects.len()).position(scroll_offset);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            area.inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut scrollbar_state,
        );
    }
}

fn render_targets(frame: &mut Frame, app: &App, area: Rect) {
    let inner = area.inner(Margin {
        vertical: 1,
        horizontal: 1,
    });
    let visible_rows = inner.height.saturating_sub(1) as usize;
    let target_offset = target_scroll_offset(app, visible_rows);

    if app.filtered_targets.is_empty() {
        frame.render_widget(
            Paragraph::new("No matching targets")
                .block(
                    Block::bordered()
                        .title("Targets")
                        .title_alignment(Alignment::Center)
                        .border_type(BorderType::Rounded),
                )
                .fg(if app.focus == Focus::Targets { ACTIVE_COLOR } else { INACTIVE_COLOR }),
            area,
        );
    } else {
        let rows: Vec<Row> = app
            .visible_targets()
            .enumerate()
            .skip(target_offset)
            .take(visible_rows)
            .map(|(index, target)| {
                let is_selected = app.focus == Focus::Targets && app.target_cursor == index as isize;
                let style = if is_selected {
                    Style::default()
                        .fg(SELECTED_TEXT_COLOR)
                        .bg(ACTIVE_COLOR)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(if app.focus == Focus::Targets {
                        ACTIVE_TEXT_COLOR
                    } else {
                        INACTIVE_TEXT_COLOR
                    })
                };
                let indicator = if let Some(project) = app.current_project() {
                    if app
                        .target_status_for(project, target)
                        .is_some_and(|status| status.kind == TargetStatusKind::Building)
                    {
                        spinner_frame(app)
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };
                Row::new(vec![indicator, target.name.clone(), target.kind.clone(), target.path.clone()]).style(style)
            })
            .collect();

        frame.render_widget(
            Table::new(
                rows,
                [Constraint::Length(2), Constraint::Fill(1), Constraint::Length(10), Constraint::Length(18)],
            )
            .header(
                Row::new(vec!["", "Name", "Kind", "Path"])
                    .style(Style::default().add_modifier(Modifier::BOLD)),
            )
            .block(
                Block::bordered()
                    .title("Targets")
                    .title_alignment(Alignment::Center)
                    .border_type(BorderType::Rounded),
            )
            .fg(if app.focus == Focus::Targets { ACTIVE_COLOR } else { INACTIVE_COLOR }),
            area,
        );
    }

    if !app.filtered_targets.is_empty() {
        let mut scrollbar_state = ScrollbarState::new(app.filtered_targets.len()).position(target_offset);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            area.inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut scrollbar_state,
        );
    }
}

fn render_search_input(frame: &mut Frame, area: Rect, input: &tui_input::Input, editing: bool, _title: &str) {
    let width = area.width.max(3) - 3;
    let scroll = input.visual_scroll(width as usize);
    let widget = Paragraph::new(input.value())
        .style(Style::default().fg(ACTIVE_COLOR))
        .scroll((0, scroll as u16))
        .block(Block::bordered().title("Search").border_type(BorderType::Rounded));
    frame.render_widget(widget, area);

    if editing {
        let x = input.visual_cursor().max(scroll) - scroll + 1;
        frame.set_cursor_position((area.x + x as u16, area.y + 1));
    }
}

fn format_size(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    if bytes == 0 {
        return "0 B".to_string();
    }
    format!("{:.1} GiB", bytes as f64 / GIB)
}

fn render_running(frame: &mut Frame, app: &App, area: Rect) {
    let rows: Vec<Row> = app
        .running_target_statuses()
        .map(|status| {
            Row::new(vec![
                format!("{}/{}", status.project_name, status.target_name),
                match status.profile {
                    RunProfile::Debug => "debug".to_string(),
                    RunProfile::Release => "release".to_string(),
                },
                format_process_stats(status),
                format_duration(status.started_at.map(|started_at| started_at.elapsed())),
            ])
            .style(Style::default().fg(ACTIVE_TEXT_COLOR))
        })
        .collect();

    frame.render_widget(
        Table::new(
            rows,
            [Constraint::Fill(1), Constraint::Length(8), Constraint::Length(24), Constraint::Length(8)],
        )
        .header(Row::new(vec!["Target", "Profile", "CPU/GPU/Mem", "Uptime"]).style(Style::default().add_modifier(Modifier::BOLD)))
        .block(
            Block::bordered()
                .title("Running")
                .title_alignment(Alignment::Center)
                .border_type(BorderType::Rounded),
        )
        .fg(ACTIVE_COLOR),
        area,
    );
}

fn spinner_frame(app: &App) -> String {
    app.spinner_state.frame_str().to_string()
}

fn format_process_stats(status: &crate::app::TargetStatus) -> String {
    let cpu = status
        .stats
        .cpu_percent
        .map(|cpu| format!("{cpu:.1}%"))
        .unwrap_or_else(|| "—".to_string());
    let mem = status
        .stats
        .memory_bytes
        .map(format_size)
        .unwrap_or_else(|| "—".to_string());
    format!("{cpu} / — / {mem}")
}

fn format_duration(duration: Option<std::time::Duration>) -> String {
    let Some(duration) = duration else {
        return "—".to_string();
    };
    let secs = duration.as_secs();
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

fn project_scroll_offset(app: &App, visible_rows: usize) -> usize {
    scroll_offset(app.filtered_projects.len(), app.cursor, visible_rows)
}

fn target_scroll_offset(app: &App, visible_rows: usize) -> usize {
    scroll_offset(app.filtered_targets.len(), app.target_cursor, visible_rows)
}

fn scroll_offset(len: usize, cursor: isize, visible_rows: usize) -> usize {
    if len == 0 || visible_rows == 0 || cursor < 0 {
        return 0;
    }

    let max_offset = len.saturating_sub(visible_rows);
    let cursor = usize::try_from(cursor).unwrap_or(0);
    let center = visible_rows / 2;

    cursor.saturating_sub(center).min(max_offset)
}
