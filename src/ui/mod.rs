use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};

use crate::{app::state::AppState, types::Event};

pub fn draw(frame: &mut Frame, app: &AppState) {
    let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(5), Constraint::Length(3)])
        .split(frame.area());

    let title = Paragraph::new("GoldBot TUI Agent · q quit · d 折叠/展开")
        .block(Block::default().borders(Borders::ALL).title("状态"));
    frame.render_widget(title, chunks[0]);

    if app.final_summary.is_some() && app.collapsed {
        let summary = app.final_summary.clone().unwrap_or_default();
        let p = Paragraph::new(summary)
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::ALL).title("最终结果（已折叠）"));
        frame.render_widget(p, chunks[1]);
    } else {
        let items: Vec<ListItem> = app
            .events
            .iter()
            .map(|e| ListItem::new(event_text(e)))
            .collect();
        let list = List::new(items).block(Block::default().borders(Borders::ALL).title("过程日志"));
        frame.render_widget(list, chunks[1]);
    }

    let footer = if app.pending_confirm.is_some() {
        "等待确认：↑/↓选择，Enter确认"
    } else if app.running {
        "运行中..."
    } else {
        "已完成，按 d 查看/隐藏过程"
    };
    frame.render_widget(
        Paragraph::new(footer).block(Block::default().borders(Borders::ALL).title("提示")),
        chunks[2],
    );

    if let Some((cmd, reason)) = &app.pending_confirm {
        let area = centered_rect(70, 40, frame.area());
        frame.render_widget(Clear, area);
        let popup_chunks = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(3),
        ])
        .split(area);

        frame.render_widget(
            Paragraph::new(format!("高风险命令：{cmd}"))
                .block(Block::default().borders(Borders::ALL).title("确认")),
            popup_chunks[0],
        );
        frame.render_widget(
            Paragraph::new(reason.clone())
                .block(Block::default().borders(Borders::ALL).title("原因")),
            popup_chunks[1],
        );

        let options = AppState::menu_options();
        let items: Vec<ListItem> = options
            .iter()
            .enumerate()
            .map(|(i, x)| {
                if i == app.selected_menu {
                    ListItem::new(format!("> {x}"))
                } else {
                    ListItem::new(format!("  {x}"))
                }
            })
            .collect();
        frame.render_widget(
            List::new(items).block(Block::default().borders(Borders::ALL).title("操作")),
            popup_chunks[2],
        );
    }
}

fn event_text(e: &Event) -> String {
    match e {
        Event::Thinking { text } => format!("[THINKING] {text}"),
        Event::ToolCall { command } => format!("[TOOL_CALL] {command}"),
        Event::ToolResult {
            command,
            exit_code,
            output,
        } => format!("[TOOL_RESULT] {command} => exit={exit_code}\n{output}"),
        Event::NeedsConfirmation { command, reason } => {
            format!("[NEEDS_CONFIRMATION] {command} ({reason})")
        }
        Event::Final { summary } => format!("[FINAL] {summary}"),
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(r);

    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(popup_layout[1])[1]
}
