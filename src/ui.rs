use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Table, Wrap,
    },
};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, CiRun, Focus, RunProfile, TargetStatusKind};

const INACTIVE_COLOR: Color = Color::Indexed(0);
const ACTIVE_COLOR: Color = Color::Indexed(201);
const ACTIVE_TEXT_COLOR: Color = Color::Indexed(7);
const INACTIVE_TEXT_COLOR: Color = Color::Indexed(8);
const SELECTED_TEXT_COLOR: Color = Color::Indexed(0);

pub fn render(frame: &mut Frame, app: &mut App) {
    let columns = Layout::horizontal([
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
    ])
    .spacing(1);

    let [left, middle, right] = frame.area().layout(&columns);

    let has_ci_runs = app.project_ci_runs().is_some();

    let has_languages = app.project_languages().is_some();

    match (has_ci_runs, has_languages) {
        (true, true) => {
            let left_chunks = Layout::vertical([
                Constraint::Percentage(34),
                Constraint::Percentage(33),
                Constraint::Percentage(33),
            ])
            .split(left);
            render_metadata(frame, app, left_chunks[0]);
            render_languages(frame, app, left_chunks[1]);
            render_ci_runs(frame, app, left_chunks[2]);
        }
        (true, false) => {
            let left_chunks =
                Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(left);
            render_metadata(frame, app, left_chunks[0]);
            render_ci_runs(frame, app, left_chunks[1]);
        }
        (false, true) => {
            let left_chunks =
                Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(left);
            render_metadata(frame, app, left_chunks[0]);
            render_languages(frame, app, left_chunks[1]);
        }
        (false, false) => render_metadata(frame, app, left),
    }

    let show_project_search =
        !app.project_query.is_empty() || (app.focus == Focus::Projects && app.filter_mode);
    let middle_chunks = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(if show_project_search { 3 } else { 0 }),
    ])
    .split(middle);
    render_projects(frame, app, middle_chunks[0]);
    if show_project_search {
        render_search_input(
            frame,
            middle_chunks[1],
            &app.project_input,
            app.filter_mode,
            "Search projects",
        );
    }

    let show_target_search =
        !app.target_query.is_empty() || (app.focus == Focus::Targets && app.filter_mode);
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
        render_search_input(
            frame,
            right_chunks[2],
            &app.target_input,
            app.filter_mode,
            "Search targets",
        );
    }
}

fn render_metadata(frame: &mut Frame, app: &mut App, area: Rect) {
    let (title, body) = if let Some(metadata) = app.project_metadata() {
        let path = app
            .current_project()
            .map(|project| project.path.display().to_string())
            .unwrap_or_else(|| "—".to_string());
        let secondary_style = Style::default().fg(INACTIVE_TEXT_COLOR);
        let body = vec![
            Line::from(Span::styled(path, secondary_style)),
            Line::default(),
            Line::from(Span::styled(metadata.description.clone(), secondary_style)),
        ];

        (
            format!(
                "{}{}{} {}",
                metadata.package_name,
                if metadata.package_version == "—" {
                    String::new()
                } else {
                    format!(" @{}", metadata.package_version)
                },
                if metadata.git_branch == "—" {
                    String::new()
                } else {
                    format!("  {}", metadata.git_branch)
                },
                metadata.git_status
            ),
            body,
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
                    .title(
                        Line::from(title).style(
                            Style::default()
                                .fg(ACTIVE_TEXT_COLOR)
                                .add_modifier(Modifier::BOLD),
                        ),
                    )
                    .title_alignment(Alignment::Center)
                    .border_type(BorderType::Rounded),
            )
            .fg(INACTIVE_COLOR),
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
                .fg(if app.focus == Focus::Projects {
                    ACTIVE_COLOR
                } else {
                    INACTIVE_COLOR
                }),
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
                let size_text = cached_size
                    .map(format_size)
                    .unwrap_or_else(|| "…".to_string());
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
                let bookmark = if app.is_bookmarked(&project) {
                    "🌟"
                } else {
                    ""
                };
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
            Table::new(
                rows,
                [
                    Constraint::Length(2),
                    Constraint::Fill(1),
                    Constraint::Length(10),
                    Constraint::Length(12),
                ],
            )
            .header(
                Row::new(vec!["", "Name", "Size", "Last Opened"])
                    .style(Style::default().add_modifier(Modifier::BOLD)),
            )
            .block(
                Block::bordered()
                    .title("Projects")
                    .title_alignment(Alignment::Center)
                    .border_type(BorderType::Rounded),
            )
            .fg(if app.focus == Focus::Projects {
                ACTIVE_COLOR
            } else {
                INACTIVE_COLOR
            }),
            area,
        );
    }

    if !app.filtered_projects.is_empty() {
        let mut scrollbar_state =
            ScrollbarState::new(app.filtered_projects.len()).position(scroll_offset);
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

    if app.targets_loading() {
        frame.render_widget(
            Paragraph::new("Loading targets…")
                .block(
                    Block::bordered()
                        .title("Targets")
                        .title_alignment(Alignment::Center)
                        .border_type(BorderType::Rounded),
                )
                .fg(if app.focus == Focus::Targets {
                    ACTIVE_COLOR
                } else {
                    INACTIVE_COLOR
                }),
            area,
        );
    } else if app.filtered_targets.is_empty() {
        frame.render_widget(
            Paragraph::new("No matching targets")
                .block(
                    Block::bordered()
                        .title("Targets")
                        .title_alignment(Alignment::Center)
                        .border_type(BorderType::Rounded),
                )
                .fg(if app.focus == Focus::Targets {
                    ACTIVE_COLOR
                } else {
                    INACTIVE_COLOR
                }),
            area,
        );
    } else {
        let visible_targets: Vec<_> = app
            .visible_targets()
            .enumerate()
            .skip(target_offset)
            .take(visible_rows)
            .map(|(index, target)| (index, target.clone()))
            .collect();

        let rows: Vec<Row> = visible_targets
            .into_iter()
            .map(|(index, target)| {
                let is_selected =
                    app.focus == Focus::Targets && app.target_cursor == index as isize;
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
                let grouped_path = if index > 0
                    && app
                        .targets
                        .get(index - 1)
                        .is_some_and(|prev| prev.path == target.path)
                {
                    String::new()
                } else {
                    target.path.clone()
                };
                Row::new(vec![target.name.clone(), target.kind.clone(), grouped_path]).style(style)
            })
            .collect();

        frame.render_widget(
            Table::new(
                rows,
                [
                    Constraint::Length(20),
                    Constraint::Length(10),
                    Constraint::Fill(1),
                ],
            )
            .header(
                Row::new(vec!["Name", "Kind", "Path"])
                    .style(Style::default().add_modifier(Modifier::BOLD)),
            )
            .block(
                Block::bordered()
                    .title("Targets")
                    .title_alignment(Alignment::Center)
                    .border_type(BorderType::Rounded),
            )
            .fg(if app.focus == Focus::Targets {
                ACTIVE_COLOR
            } else {
                INACTIVE_COLOR
            }),
            area,
        );
    }

    if !app.filtered_targets.is_empty() {
        let mut scrollbar_state =
            ScrollbarState::new(app.filtered_targets.len()).position(target_offset);
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

fn render_search_input(
    frame: &mut Frame,
    area: Rect,
    input: &tui_input::Input,
    editing: bool,
    _title: &str,
) {
    let width = area.width.max(3) - 3;
    let scroll = input.visual_scroll(width as usize);
    let widget = Paragraph::new(input.value())
        .style(Style::default().fg(Color::Green))
        .scroll((0, scroll as u16))
        .block(
            Block::bordered()
                .title("Search")
                .border_type(BorderType::Rounded),
        );
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
            let indicator = if status.kind == TargetStatusKind::Building {
                spinner_frame(app)
            } else {
                String::new()
            };
            Row::new(vec![
                indicator,
                format!("{}/{}", status.project_name, status.target_name),
                match status.profile {
                    RunProfile::Debug => "debug".to_string(),
                    RunProfile::Release => "release".to_string(),
                },
                format_duration(status.started_at.map(|started_at| started_at.elapsed())),
            ])
            .style(Style::default().fg(ACTIVE_TEXT_COLOR))
        })
        .collect();

    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(2),
                Constraint::Fill(1),
                Constraint::Length(8),
                Constraint::Length(8),
            ],
        )
        .header(
            Row::new(vec!["", "Target", "Profile", "Uptime"])
                .style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .block(
            Block::bordered()
                .title("Running")
                .title_alignment(Alignment::Center)
                .border_type(BorderType::Rounded),
        )
        .fg(Color::Green),
        area,
    );
}

fn spinner_frame(app: &App) -> String {
    app.spinner_state.frame_str().to_string()
}

fn format_duration(duration: Option<std::time::Duration>) -> String {
    let Some(duration) = duration else {
        return "—".to_string();
    };
    let secs = duration.as_secs();
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

fn render_languages(frame: &mut Frame, app: &App, area: Rect) {
    let Some(languages) = app.project_languages() else {
        return;
    };

    let rows: Vec<Row> = languages
        .languages
        .iter()
        .take(area.height.saturating_sub(3) as usize)
        .map(|language| {
            Row::new(vec![
                Cell::from(language_label_text(&language.name)).style(
                    Style::default()
                        .fg(ACTIVE_TEXT_COLOR)
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::from(format_count(language.code)),
                Cell::from(format_count(language.comments)),
                Cell::from(format_count(language.blanks)),
            ])
            .style(Style::default().fg(INACTIVE_TEXT_COLOR))
        })
        .collect();

    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Fill(1),
                Constraint::Length(6),
                Constraint::Length(8),
                Constraint::Length(6),
            ],
        )
        .header(
            Row::new(vec!["Language", "Code", "Comments", "Blank"]).style(
                Style::default()
                    .fg(ACTIVE_TEXT_COLOR)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .column_spacing(1)
        .block(
            Block::bordered()
                .title("Languages")
                .title_alignment(Alignment::Center)
                .border_type(BorderType::Rounded),
        )
        .fg(INACTIVE_COLOR),
        area,
    );
}

fn render_ci_runs(frame: &mut Frame, app: &App, area: Rect) {
    if app.project_ci_runs().is_none() {
        return;
    }

    let inner = area.inner(Margin {
        vertical: 1,
        horizontal: 1,
    });
    let visible_rows = inner.height.saturating_sub(1) as usize;
    let scroll_offset = ci_scroll_offset(app, visible_rows);

    let rows: Vec<Row> = app
        .visible_ci_runs()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_rows)
        .map(|(index, run)| ci_run_row(app, index, run))
        .collect();

    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(3),
                Constraint::Length(10),
                Constraint::Fill(1),
                Constraint::Length(16),
            ],
        )
        .header(
            Row::new(vec!["", "Branch", "Commit", "Timestamp"])
                .style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .block(
            Block::bordered()
                .title("CI")
                .title_alignment(Alignment::Center)
                .border_type(BorderType::Rounded),
        )
        .fg(if app.focus == Focus::CiRuns {
            ACTIVE_COLOR
        } else {
            INACTIVE_COLOR
        }),
        area,
    );

    let mut scrollbar_state =
        ScrollbarState::new(app.filtered_ci_runs.len()).position(scroll_offset);
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight),
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut scrollbar_state,
    );
}

fn ci_run_row(app: &App, index: usize, run: &CiRun) -> Row<'static> {
    let is_selected = app.focus == Focus::CiRuns && app.ci_cursor == index as isize;
    let row_style = if is_selected {
        Style::default()
            .fg(SELECTED_TEXT_COLOR)
            .bg(ACTIVE_COLOR)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(if app.focus == Focus::CiRuns {
            ACTIVE_TEXT_COLOR
        } else {
            INACTIVE_TEXT_COLOR
        })
    };
    let status = match run.status.as_str() {
        "success" => Span::styled("✓", row_style.fg(Color::Green)),
        "failure" => Span::styled("✗", row_style.fg(Color::Red)),
        "cancelled" => Span::styled("○", row_style.fg(Color::Yellow)),
        _ => Span::styled("…", row_style.fg(Color::Blue)),
    };

    Row::new(vec![
        Cell::from(Line::from(status)),
        Cell::from(truncate(&run.branch, 10)),
        Cell::from(truncate(&run.title, 48)),
        Cell::from(format_ci_time(&run.created_at)),
    ])
    .style(row_style)
}

fn language_label(language: &str) -> Line<'static> {
    match language.to_ascii_lowercase().as_str() {
        "rust" => language_label_parts("🦀", language, false),
        "c" | "c header" => language_label_parts("", language, false),
        "c++" | "c++ header" | "c++ module" => language_label_parts("", language, false),
        "java" => language_label_parts("☕", language, false),
        "go" => language_label_parts("", language, false),
        "python" => language_label_parts("🐍", language, false),
        "javascript" | "jsx" => language_label_parts("", language, false),
        "typescript" | "tsx" => language_label_parts("", language, false),
        "markdown" => language_label_parts("", language, false),
        "shell" | "bash" | "zsh" | "fish" => language_label_parts("", language, false),
        "liquid" => language_label_parts("💧", language, false),
        "toml" => language_label_parts("⚙️", language, false),
        "json" => language_label_parts("", language, false),
        "html" => language_label_parts("🌐", language, false),
        "plain text" => language_label_parts("📄", language, false),
        "xml" => language_label_parts("󰗀", language, false),
        "glsl" | "webgpu shader language" => language_label_parts("🔺", language, false),
        "svg" => language_label_parts("📐", language, false),
        "yaml" => language_label_parts("", language, false),
        "bitbake" => language_label_parts("🍞", language, false),
        "cmake" => language_label_parts("△", language, true),
        "makefile" => language_label_parts("🛠️", language, false),
        "autoconf" => language_label_parts("🔧", language, false),
        "asciidoc" => language_label_parts("󱈙", language, false),
        "batch" => language_label_parts("󰆍", language, false),
        "rusty object notation" => language_label_parts("󰘦", language, false),
        _ => language_label_parts("", language, false),
    }
}

fn language_label_text(language: &str) -> String {
    let line = language_label(language);
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

fn language_label_parts(prefix: &str, language: &str, bright: bool) -> Line<'static> {
    let prefix_style = if bright {
        Style::default()
            .fg(ACTIVE_TEXT_COLOR)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let width = prefix.width();
    let gap = " ".repeat((3usize.saturating_sub(width)).max(1));
    Line::from(vec![
        Span::styled(format!("{prefix}{gap}"), prefix_style),
        Span::raw(language.to_string()),
    ])
}

fn format_count(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}m", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}k", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    value
        .chars()
        .take(max.saturating_sub(1))
        .collect::<String>()
        + "…"
}

fn format_ci_time(value: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|_| value.to_string())
}

fn ci_scroll_offset(app: &App, visible_rows: usize) -> usize {
    scroll_offset(app.filtered_ci_runs.len(), app.ci_cursor, visible_rows)
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
