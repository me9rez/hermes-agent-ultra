use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonaBlockKind {
    Instruction,
    Terminology,
    Examples,
    StyleHint,
    OutputDirective,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonaBlock {
    pub kind: PersonaBlockKind,
    pub variants: HashMap<String, String>,
    pub follow_user_locale: bool,
}
