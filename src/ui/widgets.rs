use super::{App, pane_border_style};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Flex, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, BorderType, Paragraph},
};

pub(super) fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let [area] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::horizontal([Constraint::Length(width.min(area.width))])
        .flex(Flex::Center)
        .areas(area);
    area
}

pub(super) fn render_search_input(
    frame: &mut Frame,
    area: Rect,
    input: &tui_input::Input,
    editing: bool,
    _title: &str,
) {
    let width = area.width.max(3) - 3;
    let scroll = input.visual_scroll(width as usize);
    let search_text_style = Style::default()
        .fg(Color::Yellow)
        .bg(Color::Reset)
        .add_modifier(ratatui::style::Modifier::BOLD);
    let widget = Paragraph::new(input.value())
        .style(search_text_style)
        .scroll((0, scroll as u16))
        .block(
            Block::bordered()
                .title("Search")
                .border_type(BorderType::Rounded)
                .style(Style::default().fg(Color::Indexed(7)).bg(Color::Reset))
                .border_style(pane_border_style(editing)),
        );
    frame.render_widget(widget, area);

    if editing {
        let x = input.visual_cursor().max(scroll) - scroll + 1;
        frame.set_cursor_position((area.x + x as u16, area.y + 1));
    }
}
pub(super) fn ci_scroll_offset(app: &App, visible_rows: usize) -> usize {
    scroll_offset(app.filtered_ci_runs.len(), app.ci_cursor, visible_rows)
}

pub(super) fn project_scroll_offset(app: &App, visible_rows: usize) -> usize {
    scroll_offset(app.filtered_projects.len(), app.cursor, visible_rows)
}

pub(super) fn target_scroll_offset(app: &App, visible_rows: usize) -> usize {
    scroll_offset(app.filtered_targets.len(), app.target_cursor, visible_rows)
}

pub(super) fn scroll_offset(len: usize, cursor: isize, visible_rows: usize) -> usize {
    if len == 0 || visible_rows == 0 || cursor < 0 {
        return 0;
    }

    let max_offset = len.saturating_sub(visible_rows);
    let cursor = usize::try_from(cursor).unwrap_or(0);
    let center = visible_rows / 2;

    cursor.saturating_sub(center).min(max_offset)
}
