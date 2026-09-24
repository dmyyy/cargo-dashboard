use super::format::{format_ci_time, format_count, language_label_text, truncate};
use super::widgets::ci_scroll_offset;
use super::{
    ACTIVE_COLOR, ACTIVE_TEXT_COLOR, App, CURSOR_COLOR, CiRun, Focus, INACTIVE_COLOR,
    INACTIVE_TEXT_COLOR, SELECTED_TEXT_COLOR, pane_border_style,
};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Margin, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Table, Wrap,
    },
};
pub(super) fn render_metadata(frame: &mut Frame, app: &mut App, area: Rect) {
    let (title, body) = if let Some(metadata) = app.project_metadata() {
        let path = app
            .current_project()
            .map(|project| project.path.display().to_string())
            .unwrap_or_else(|| "—".to_string());
        let secondary_style = Style::default().fg(INACTIVE_TEXT_COLOR);
        let mut body = vec![
            Line::from(Span::styled(path, secondary_style)),
            Line::default(),
            Line::from(Span::styled(metadata.description.clone(), secondary_style)),
        ];

        if let Some(description) = app.current_target_description() {
            body.push(Line::default());
            body.push(Line::from(vec![
                Span::styled("Target: ", Style::default().fg(ACTIVE_TEXT_COLOR)),
                Span::styled(description.to_string(), secondary_style),
            ]));
        }

        (
            format!(
                "{}{}{}",
                metadata.package_name,
                if metadata.package_version == "—" {
                    String::new()
                } else {
                    format!(" v{}", metadata.package_version)
                },
                if metadata.git_branch == "—" {
                    String::new()
                } else {
                    format!("  {}", metadata.git_branch)
                }
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
            .alignment(Alignment::Center)
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

pub(super) fn render_languages(frame: &mut Frame, app: &App, area: Rect) {
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
                    .fg(ACTIVE_COLOR)
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

pub(super) fn render_ci_runs(frame: &mut Frame, app: &App, area: Rect) {
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
                .border_type(BorderType::Rounded)
                .border_style(pane_border_style(app.focus == Focus::CiRuns)),
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
            .bg(CURSOR_COLOR)
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
