mod agent;
mod memory;
mod app;
mod safety;
mod tools;
mod types;
mod ui;

use std::{io, time::Duration};

use agent::r#loop::plan_from_codex_or_sample;
use memory::store::MemoryStore;
use app::state::AppState;
use crossterm::{
    event::{self, Event as CEvent, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, prelude::CrosstermBackend};
use safety::policy::{RiskLevel, assess_command};
use types::Event;

fn main() -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> anyhow::Result<()> {
    let task = std::env::var("GOLDBOT_TASK").unwrap_or_else(|_| "整理当前目录并汇总文件信息".to_string());
    let mut app = AppState::new(plan_from_codex_or_sample(), task);

    loop {
        terminal.draw(|f| ui::draw(f, &app))?;

        if app.running && app.pending_confirm.is_none() {
            run_one_step(&mut app);
            let _ = app.compactor.tick_and_maybe_compact(&mut app.events);
        }

        if event::poll(Duration::from_millis(120))?
            && let CEvent::Key(key) = event::read()?
        {
            match key.code {
                KeyCode::Char('q') => break,
                KeyCode::Char('d') => {
                    if app.final_summary.is_some() {
                        app.collapsed = !app.collapsed;
                    }
                }
                KeyCode::Up => {
                    if app.pending_confirm.is_some() {
                        app.selected_menu = app.selected_menu.saturating_sub(1);
                    }
                }
                KeyCode::Down => {
                    if app.pending_confirm.is_some() {
                        app.selected_menu = (app.selected_menu + 1).min(3);
                    }
                }
                KeyCode::Enter => {
                    if let Some((cmd, _reason)) = app.pending_confirm.clone() {
                        match app.selected_choice() {
                            types::ConfirmationChoice::Execute => execute_command(&mut app, &cmd),
                            types::ConfirmationChoice::Edit => {
                                app.events.push(Event::ToolResult {
                                    command: cmd,
                                    exit_code: 0,
                                    output: "Edit 暂未实现（MVP）".into(),
                                });
                                app.pending_confirm = None;
                                app.index += 1;
                            }
                            types::ConfirmationChoice::Skip => {
                                app.events.push(Event::ToolResult {
                                    command: cmd,
                                    exit_code: 0,
                                    output: "用户选择跳过".into(),
                                });
                                app.pending_confirm = None;
                                app.index += 1;
                            }
                            types::ConfirmationChoice::Abort => {
                                finish(&mut app, "任务被用户终止".into());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    Ok(())
}

fn run_one_step(app: &mut AppState) {
    if app.index >= app.plan.len() {
        finish(app, "我已经整理完毕（MVP 演示流程完成）".into());
        return;
    }

    let step = app.plan[app.index].clone();
    app.events.push(Event::Thinking {
        text: step.thought.clone(),
    });

    let (risk, reason) = assess_command(&step.command);
    match risk {
        RiskLevel::Safe => execute_command(app, &step.command),
        RiskLevel::Confirm => {
            app.events.push(Event::NeedsConfirmation {
                command: step.command.clone(),
                reason: reason.clone(),
            });
            app.pending_confirm = Some((step.command.clone(), reason));
            app.selected_menu = 0;
        }
        RiskLevel::Block => {
            app.events.push(Event::ToolResult {
                command: step.command.clone(),
                exit_code: -1,
                output: "已拦截高危命令".into(),
            });
            app.index += 1;
        }
    }
}

fn execute_command(app: &mut AppState, cmd: &str) {
    app.events.push(Event::ToolCall {
        command: cmd.to_string(),
    });
    match tools::runner::run_command(cmd) {
        Ok(out) => app.events.push(Event::ToolResult {
            command: cmd.to_string(),
            exit_code: out.exit_code,
            output: out.output,
        }),
        Err(e) => app.events.push(Event::ToolResult {
            command: cmd.to_string(),
            exit_code: -1,
            output: format!("执行失败: {e}"),
        }),
    }

    app.pending_confirm = None;
    app.index += 1;
}

fn finish(app: &mut AppState, summary: String) {
    app.events.push(Event::Final {
        summary: summary.clone(),
    });
    let store = MemoryStore::new();
    let _ = store.append_short_term(&app.task, &summary);
    let _ = store.append_long_term(&format!("task= {}; final= {}", app.task, summary));

    app.running = false;
    app.final_summary = Some(summary);
    app.collapsed = true;
    app.pending_confirm = None;
}
