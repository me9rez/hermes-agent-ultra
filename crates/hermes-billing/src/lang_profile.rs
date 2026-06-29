use std::cmp::Ordering;
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use hermes_tasks::TaskCategory;
use serde::{Deserialize, Serialize};

use crate::heuristic;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    En,
    ZhCN,
    ZhHant,
    Ja,
}

impl Language {
    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag.trim().to_ascii_lowercase().as_str() {
            "en" => Some(Self::En),
            "zh-cn" | "zh_cn" | "zh" => Some(Self::ZhCN),
            "zh-hant" | "zh_hant" | "zh-tw" | "zh-hk" => Some(Self::ZhHant),
            "ja" | "ja-jp" => Some(Self::Ja),
            _ => None,
        }
    }

    pub fn tag(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::ZhCN => "zh-CN",
            Self::ZhHant => "zh-Hant",
            Self::Ja => "ja",
        }
    }
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

impl ProfileSource {
    pub fn priority(self) -> u8 {
        match self {
            Self::ManualOverride => 5,
            Self::Telemetry => 4,
            Self::VendorClaim => 3,
            Self::ProbeBenchmark => 2,
            Self::NamingHeuristic => 1,
        }
    }
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

pub const LOW_CONFIDENCE_THRESHOLD: f32 = 0.5;
pub const TELEMETRY_MIN_SAMPLES: u64 = 1000;

#[derive(Debug, Clone)]
pub struct AutoBlendContext {
    pub model_profile: ModelLanguageProfile,
    pub user_locale: Language,
    pub vertical_task_category: TaskCategory,
}

pub fn is_low_confidence(profile: &ModelLanguageProfile) -> bool {
    profile.confidence < LOW_CONFIDENCE_THRESHOLD
}

pub fn telemetry_eligible(profile: &ModelLanguageProfile) -> bool {
    profile.source == ProfileSource::Telemetry && profile.sample_count >= TELEMETRY_MIN_SAMPLES
}

pub fn resolve_profile(candidates: &[ModelLanguageProfile]) -> Option<ModelLanguageProfile> {
    candidates
        .iter()
        .max_by(|a, b| {
            a.source
                .priority()
                .cmp(&b.source.priority())
                .then_with(|| {
                    a.confidence
                        .partial_cmp(&b.confidence)
                        .unwrap_or(Ordering::Equal)
                })
                .then_with(|| a.sample_count.cmp(&b.sample_count))
        })
        .cloned()
}

pub fn default_profile(model_id: &str) -> ModelLanguageProfile {
    match model_id {
        "tongyi-qwen-max" => ModelLanguageProfile {
            model_id: model_id.into(),
            primary_lang: Language::ZhCN,
            language_scores: HashMap::from([
                (Language::ZhCN, 1.0),
                (Language::En, 0.85),
                (Language::Ja, 0.6),
            ]),
            task_overrides: HashMap::from([
                (TaskCategory::Code, Language::En),
                (TaskCategory::Math, Language::En),
            ]),
            source: ProfileSource::ManualOverride,
            confidence: 0.9,
            sample_count: 0,
            last_updated: Utc::now(),
        },
        "openai-gpt-5" => ModelLanguageProfile {
            model_id: model_id.into(),
            primary_lang: Language::En,
            language_scores: HashMap::from([
                (Language::En, 1.0),
                (Language::ZhCN, 0.94),
                (Language::Ja, 0.92),
            ]),
            task_overrides: HashMap::new(),
            source: ProfileSource::ManualOverride,
            confidence: 0.95,
            sample_count: 0,
            last_updated: Utc::now(),
        },
        "kimi-k2" => ModelLanguageProfile {
            model_id: model_id.into(),
            primary_lang: Language::ZhCN,
            language_scores: HashMap::from([(Language::ZhCN, 0.95), (Language::En, 0.78)]),
            task_overrides: HashMap::from([(TaskCategory::Code, Language::En)]),
            source: ProfileSource::ManualOverride,
            confidence: 0.85,
            sample_count: 0,
            last_updated: Utc::now(),
        },
        "deepseek-r1" => ModelLanguageProfile {
            model_id: model_id.into(),
            primary_lang: Language::ZhCN,
            language_scores: HashMap::from([(Language::ZhCN, 0.95), (Language::En, 0.92)]),
            task_overrides: HashMap::from([
                (TaskCategory::Code, Language::En),
                (TaskCategory::Math, Language::En),
            ]),
            source: ProfileSource::ManualOverride,
            confidence: 0.9,
            sample_count: 0,
            last_updated: Utc::now(),
        },
        other => heuristic::infer_from_model_id(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_profile_prefers_manual_override() {
        let manual = default_profile("tongyi-qwen-max");
        let heuristic = heuristic::infer_from_model_id("tongyi-qwen-max");
        let resolved = resolve_profile(&[heuristic, manual.clone()]).expect("profile");
        assert_eq!(resolved.source, ProfileSource::ManualOverride);
        assert_eq!(resolved.primary_lang, Language::ZhCN);
    }

    #[test]
    fn low_confidence_unknown_model() {
        let profile = heuristic::infer_from_model_id("unknown-model-v9");
        assert!(is_low_confidence(&profile));
    }
}
