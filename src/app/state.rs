use crate::agent::r#loop::PlanStep;
use crate::types::{ConfirmationChoice, Event};

pub struct AppState {
    pub events: Vec<Event>,
    pub plan: Vec<PlanStep>,
    pub index: usize,
    pub running: bool,
    pub collapsed: bool,
    pub selected_menu: usize,
    pub pending_confirm: Option<(String, String)>,
    pub final_summary: Option<String>,
}

impl AppState {
    pub fn new(plan: Vec<PlanStep>) -> Self {
        Self {
            events: Vec::new(),
            plan,
            index: 0,
            running: true,
            collapsed: false,
            selected_menu: 0,
            pending_confirm: None,
            final_summary: None,
        }
    }

    pub fn menu_options() -> [&'static str; 4] {
        ["✅ 执行", "✏️ 修改命令", "⏭ 跳过", "🛑 终止"]
    }

    pub fn selected_choice(&self) -> ConfirmationChoice {
        match self.selected_menu {
            0 => ConfirmationChoice::Execute,
            1 => ConfirmationChoice::Edit,
            2 => ConfirmationChoice::Skip,
            _ => ConfirmationChoice::Abort,
        }
    }
}
