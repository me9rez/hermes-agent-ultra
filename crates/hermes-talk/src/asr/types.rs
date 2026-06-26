#[derive(Debug, Clone)]
pub enum AsrEvent {
    /// `text` = incremental (`new_result`); `full` = SDK cumulative hypothesis (`result`).
    Partial {
        text: String,
        full: Option<String>,
    },
    Final {
        text: String,
    },
    TaskStarted,
    TaskFailed {
        message: String,
    },
}
