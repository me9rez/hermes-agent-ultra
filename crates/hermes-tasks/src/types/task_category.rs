use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskCategory {
    GeneralChat,
    Reasoning,
    Code,
    Math,
    Financial,
    Medical,
    Legal,
    Educational,
    CreativeWriting,
    Translation,
    ChineseLiterature,
    EnglishLiterature,
}

impl TaskCategory {
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "generalchat" | "general_chat" => Some(Self::GeneralChat),
            "reasoning" => Some(Self::Reasoning),
            "code" => Some(Self::Code),
            "math" => Some(Self::Math),
            "financial" => Some(Self::Financial),
            "medical" => Some(Self::Medical),
            "legal" => Some(Self::Legal),
            "educational" => Some(Self::Educational),
            "creativewriting" | "creative_writing" => Some(Self::CreativeWriting),
            "translation" => Some(Self::Translation),
            "chineseliterature" | "chinese_literature" => Some(Self::ChineseLiterature),
            "englishliterature" | "english_literature" => Some(Self::EnglishLiterature),
            _ => None,
        }
    }
}
