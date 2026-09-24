use super::format::format_duration;
use super::{
    ACTIVE_COLOR, ACTIVE_TEXT_COLOR, App, CURSOR_COLOR, Focus, INACTIVE_COLOR, INACTIVE_TEXT_COLOR,
    RunProfile, SELECTED_TEXT_COLOR, TargetStatusKind, pane_border_style,
};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Margin, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Cell, Paragraph, Row, Table},
};
use tui_term::widget::PseudoTerminal;
pub(super) fn render_running(frame: &mut Frame, app: &App, area: Rect) {
    let rows: Vec<Row> = app
        .running_target_statuses()
        .enumerate()
        .map(|(index, status)| {
            let is_selected =
                app.focus == Focus::RunningTargets && app.running_cursor == index as isize;
            let row_style = if is_selected {
                Style::default()
                    .fg(SELECTED_TEXT_COLOR)
                    .bg(CURSOR_COLOR)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(if app.focus == Focus::RunningTargets {
                    ACTIVE_TEXT_COLOR
                } else {
                    INACTIVE_TEXT_COLOR
                })
            };
            let indicator = match status.kind {
                TargetStatusKind::Failed => {
                    Cell::from(Line::from(Span::styled("✗", row_style.fg(Color::Red))))
                }
                TargetStatusKind::Building | TargetStatusKind::Running => {
                    Cell::from(spinner_frame(app))
                }
            };
            Row::new(vec![
                indicator,
                Cell::from(format!("{}/{}", status.project_name, status.target_name)),
                Cell::from(match status.profile {
                    RunProfile::Debug => "debug".to_string(),
                    RunProfile::Release => "release".to_string(),
                }),
                Cell::from(format_duration(
                    status.started_at.map(|started_at| started_at.elapsed()),
                )),
            ])
            .style(row_style)
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
                .border_type(BorderType::Rounded)
                .border_style(pane_border_style(app.focus == Focus::RunningTargets)),
        )
        .fg(if app.focus == Focus::RunningTargets {
            ACTIVE_COLOR
        } else {
            INACTIVE_COLOR
        }),
        area,
    );
}

pub(super) fn render_running_terminal(frame: &mut Frame, app: &mut App, area: Rect) {
    let inner = area.inner(Margin {
        vertical: 1,
        horizontal: 1,
    });
    app.resize_selected_running_terminal(inner.width, inner.height);

    let block = Block::bordered()
        .title("Terminal")
        .title_alignment(Alignment::Center)
        .border_type(BorderType::Rounded)
        .border_style(pane_border_style(true));

    if let Some(parser) = app.selected_running_terminal_parser()
        && let Ok(parser) = parser.lock()
    {
        frame.render_widget(PseudoTerminal::new(parser.screen()).block(block), area);
    } else {
        frame.render_widget(
            Paragraph::new("No running target selected").block(block),
            area,
        );
    }
}

fn spinner_frame(app: &App) -> String {
    app.spinner_state.frame_str().to_string()
}
