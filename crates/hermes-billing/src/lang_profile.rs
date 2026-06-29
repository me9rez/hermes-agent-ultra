use std::collections::HashMap;

use chrono::{DateTime, Utc};
use hermes_tasks::TaskCategory;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    En,
    ZhCN,
    ZhHant,
    Ja,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileSource {
    ManualOverride,
    Telemetry,
    VendorClaim,
    NamingHeuristic,
    ProbeBenchmark,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelLanguageProfile {
    pub model_id: String,
    pub primary_lang: Language,
    pub language_scores: HashMap<Language, f32>,
    pub task_overrides: HashMap<TaskCategory, Language>,
    pub source: ProfileSource,
    pub confidence: f32,
    pub sample_count: u64,
    pub last_updated: DateTime<Utc>,
}
