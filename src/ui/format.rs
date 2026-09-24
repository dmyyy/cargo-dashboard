use super::ACTIVE_TEXT_COLOR;
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;
pub(super) fn format_size(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    if bytes == 0 {
        return "0 B".to_string();
    }
    format!("{:.1} GiB", bytes as f64 / GIB)
}
pub(super) fn format_duration(duration: Option<std::time::Duration>) -> String {
    let Some(duration) = duration else {
        return "—".to_string();
    };
    let secs = duration.as_secs();
    format!("{:02}:{:02}", secs / 60, secs % 60)
}
pub(super) fn language_label(language: &str) -> Line<'static> {
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

pub(super) fn language_label_text(language: &str) -> String {
    let line = language_label(language);
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

pub(super) fn language_label_parts(prefix: &str, language: &str, bright: bool) -> Line<'static> {
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

pub(super) fn format_count(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}m", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}k", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

pub(super) fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    value
        .chars()
        .take(max.saturating_sub(1))
        .collect::<String>()
        + "…"
}

pub(super) fn format_ci_time(value: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|_| value.to_string())
}
