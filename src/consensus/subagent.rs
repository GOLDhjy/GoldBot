use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use anyhow::Result;

use crate::{consensus::engine::GeRuntime, types::Mode};

const SUBAGENT_LOOP_INTERVAL: Duration = Duration::from_millis(120);
const RUN_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(3 * 60);
const HEARTBEAT_POLL_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Debug, Clone)]
pub enum GeAgentCommand {
    InterviewReply(String),
    ReplanTodos,
    ExpandLastPrompt,
    ExpandLastResult,
    Exit,
}

#[derive(Debug, Clone)]
pub enum GeAgentEvent {
    OutputLines(Vec<String>),
    ModeChanged(Mode),
    Exited,
    Error(String),
}

pub struct GeSubagent {
    cmd_tx: Sender<GeAgentCommand>,
    evt_rx: Receiver<GeAgentEvent>,
    cancel_flag: Arc<AtomicBool>,
}

impl GeSubagent {
    pub fn start(cwd: PathBuf, initial_payload: &str) -> Result<Self> {
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let (runtime, initial_lines) = GeRuntime::enter(cwd, initial_payload, cancel_flag.clone())?;
        let initial_mode = runtime.mode();
        let (cmd_tx, cmd_rx) = mpsc::channel::<GeAgentCommand>();
        let (evt_tx, evt_rx) = mpsc::channel::<GeAgentEvent>();
        send_lines(&evt_tx, initial_lines);
        let _ = evt_tx.send(GeAgentEvent::ModeChanged(initial_mode));

        thread::spawn(move || run_worker(runtime, cmd_rx, evt_tx));

        Ok(Self {
            cmd_tx,
            evt_rx,
            cancel_flag,
        })
    }

    pub fn send(&self, cmd: GeAgentCommand) -> bool {
        self.cmd_tx.send(cmd).is_ok()
    }

    pub fn hard_exit(&self) -> bool {
        self.cancel_flag.store(true, Ordering::SeqCst);
        self.cmd_tx.send(GeAgentCommand::Exit).is_ok()
    }

    pub fn try_recv(&self) -> std::result::Result<GeAgentEvent, TryRecvError> {
        self.evt_rx.try_recv()
    }
}

fn run_worker(
    mut runtime: GeRuntime,
    cmd_rx: Receiver<GeAgentCommand>,
    evt_tx: Sender<GeAgentEvent>,
) {
    let mut last_mode = runtime.mode();
    loop {
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                GeAgentCommand::InterviewReply(text) => {
                    let long_wait = runtime.interview_needs_long_wait();
                    if long_wait {
                        let _ = evt_tx.send(GeAgentEvent::OutputLines(vec![
                            "  GE: Planning...".to_string(),
                        ]));
                    }

                    let result = runtime.handle_interview_reply(&text);
                    match result {
                        Ok((handled, lines)) => {
                            if handled {
                                send_lines(&evt_tx, lines);
                            }
                        }
                        Err(e) => {
                            let _ = evt_tx.send(GeAgentEvent::Error(format!(
                                "GE interview handling failed: {e}"
                            )));
                        }
                    }
                }
                GeAgentCommand::ReplanTodos => match runtime.replan_todos() {
                    Ok((_, lines)) => send_lines(&evt_tx, lines),
                    Err(e) => {
                        let _ = evt_tx.send(GeAgentEvent::Error(format!("GE replan failed: {e}")));
                    }
                },
                GeAgentCommand::ExpandLastPrompt => {
                    let lines = runtime.expand_last_prompt();
                    send_lines(&evt_tx, lines);
                }
                GeAgentCommand::ExpandLastResult => {
                    let lines = runtime.expand_last_result();
                    send_lines(&evt_tx, lines);
                }
                GeAgentCommand::Exit => {
                    let lines = runtime.exit();
                    send_lines(&evt_tx, lines);
                    let _ = evt_tx.send(GeAgentEvent::ModeChanged(Mode::Normal));
                    let _ = evt_tx.send(GeAgentEvent::Exited);
                    return;
                }
            }
            sync_mode(&mut last_mode, runtime.mode(), &evt_tx);
        }

        let progress_tx = evt_tx.clone();
        let heartbeat_stop = Arc::new(AtomicBool::new(false));
        let heartbeat_join = if runtime.mode() == Mode::GeRun {
            let stop = heartbeat_stop.clone();
            let tx = evt_tx.clone();
            Some(thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    if wait_or_stop(&stop, RUN_HEARTBEAT_INTERVAL) {
                        break;
                    }
                    let _ = tx.send(GeAgentEvent::OutputLines(vec![
                        "  GE: Working on current todo... (heartbeat every 3 minutes)".to_string(),
                    ]));
                }
            }))
        } else {
            None
        };

        let tick_result = runtime.tick_with_emit(|line| {
            let _ = progress_tx.send(GeAgentEvent::OutputLines(vec![line]));
        });
        heartbeat_stop.store(true, Ordering::SeqCst);
        if let Some(join) = heartbeat_join {
            let _ = join.join();
        }

        if let Err(e) = tick_result {
            let _ = evt_tx.send(GeAgentEvent::Error(format!("GE tick failed: {e}")));
            let _ = evt_tx.send(GeAgentEvent::Exited);
            return;
        }
        sync_mode(&mut last_mode, runtime.mode(), &evt_tx);
        thread::sleep(SUBAGENT_LOOP_INTERVAL);
    }
}

fn send_lines(tx: &Sender<GeAgentEvent>, lines: Vec<String>) {
    if lines.is_empty() {
        return;
    }
    let _ = tx.send(GeAgentEvent::OutputLines(lines));
}

fn sync_mode(last_mode: &mut Mode, mode: Mode, tx: &Sender<GeAgentEvent>) {
    if *last_mode == mode {
        return;
    }
    *last_mode = mode;
    let _ = tx.send(GeAgentEvent::ModeChanged(mode));
}

fn wait_or_stop(stop: &AtomicBool, duration: Duration) -> bool {
    let mut waited = Duration::ZERO;
    while waited < duration {
        if stop.load(Ordering::SeqCst) {
            return true;
        }
        let remain = duration.saturating_sub(waited);
        let sleep_for = remain.min(HEARTBEAT_POLL_INTERVAL);
        thread::sleep(sleep_for);
        waited += sleep_for;
    }
    stop.load(Ordering::SeqCst)
}
