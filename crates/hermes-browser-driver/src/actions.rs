use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserAction {
    Navigate { url: String },
    Click { selector: String },
    Type { selector: String, text: String },
    Scroll { delta_y: i32 },
    Screenshot,
    ExtractText { selector: String },
    Wait { ms: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserObservation {
    pub url: String,
    pub title: String,
    pub screenshot: Option<Vec<u8>>,
    pub dom_snapshot: Option<String>,
    pub success: bool,
    pub error: Option<String>,
}
