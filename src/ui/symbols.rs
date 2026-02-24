pub(crate) struct Symbols {
    pub spinner_frames: &'static [&'static str],
    pub prompt: &'static str,
    pub check: &'static str,
    pub running: &'static str,
    pub pending: &'static str,
    pub arrow_down: &'static str,
    pub arrow_right: &'static str,
    pub ellipsis: &'static str,
    pub dot: &'static str,
    pub record: &'static str,
    pub corner: &'static str,
    pub bullet: &'static str,
    pub warning: &'static str,
}

impl Symbols {
    pub fn current() -> &'static Self {
        #[cfg(windows)]
        {
            let is_modern = std::env::var("WT_SESSION").is_ok()
                || std::env::var("TERM_PROGRAM").is_ok()
                || std::env::var("ALACRITTY_WINDOW_ID").is_ok();
            if !is_modern {
                return &ASCII_SYMBOLS;
            }
        }
        &UNICODE_SYMBOLS
    }
}

const UNICODE_SYMBOLS: Symbols = Symbols {
    spinner_frames: &["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"],
    prompt: "❯",
    check: "✅",
    running: "◎",
    pending: "○",
    arrow_down: "▼",
    arrow_right: "⏵",
    ellipsis: "…",
    dot: "·",
    record: "⏺",
    corner: "⎿",
    bullet: "•",
    warning: "⚠",
};

const ASCII_SYMBOLS: Symbols = Symbols {
    spinner_frames: &["|", "/", "-", "\\"],
    prompt: ">",
    check: "[x]",
    running: "[~]",
    pending: "[ ]",
    arrow_down: "v",
    arrow_right: ">",
    ellipsis: "...",
    dot: "-",
    record: "*",
    corner: "\\",
    bullet: "*",
    warning: "!",
};
