use std::collections::HashMap;

use chrono::Utc;
use hermes_tasks::TaskCategory;

use crate::lang_profile::{Language, ModelLanguageProfile, ProfileSource};

const NAMING_HEURISTIC_CONFIDENCE: f32 = 0.55;

fn zh_primary_scores() -> HashMap<Language, f32> {
    HashMap::from([
        (Language::ZhCN, 1.0),
        (Language::En, 0.85),
        (Language::Ja, 0.6),
        (Language::ZhHant, 0.95),
    ])
}

fn en_primary_scores() -> HashMap<Language, f32> {
    HashMap::from([
        (Language::En, 1.0),
        (Language::ZhCN, 0.85),
        (Language::Ja, 0.75),
        (Language::ZhHant, 0.82),
    ])
}

fn code_math_overrides() -> HashMap<TaskCategory, Language> {
    HashMap::from([
        (TaskCategory::Code, Language::En),
        (TaskCategory::Math, Language::En),
    ])
}

fn is_zh_family_model(id: &str) -> bool {
    id.contains("qwen")
        || id.contains("kimi")
        || id.contains("baichuan")
        || id.contains("yi-")
        || id.contains("deepseek")
        || id.contains("tongyi")
        || id.contains("glm")
        || id.contains("ernie")
        || id.contains("hunyuan")
}

fn is_en_family_model(id: &str) -> bool {
    id.contains("gpt")
        || id.contains("claude")
        || id.contains("gemini")
        || id.contains("llama")
        || id.contains("mistral")
        || id.contains("openai")
        || id.contains("anthropic")
        || id.contains("o1")
        || id.contains("o3")
}

pub fn infer_from_model_id(model_id: &str) -> ModelLanguageProfile {
    let id = model_id.to_ascii_lowercase();
    let now = Utc::now();

    if is_zh_family_model(&id) {
        return ModelLanguageProfile {
            model_id: model_id.to_string(),
            primary_lang: Language::ZhCN,
            language_scores: zh_primary_scores(),
            task_overrides: code_math_overrides(),
            source: ProfileSource::NamingHeuristic,
            confidence: NAMING_HEURISTIC_CONFIDENCE,
            sample_count: 0,
            last_updated: now,
        };
    }

    if is_en_family_model(&id) {
        return ModelLanguageProfile {
            model_id: model_id.to_string(),
            primary_lang: Language::En,
            language_scores: en_primary_scores(),
            task_overrides: HashMap::new(),
            source: ProfileSource::NamingHeuristic,
            confidence: NAMING_HEURISTIC_CONFIDENCE,
            sample_count: 0,
            last_updated: now,
        };
    }

    ModelLanguageProfile {
        model_id: model_id.to_string(),
        primary_lang: Language::En,
        language_scores: HashMap::from([(Language::En, 0.5)]),
        task_overrides: HashMap::new(),
        source: ProfileSource::NamingHeuristic,
        confidence: 0.3,
        sample_count: 0,
        last_updated: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qwen_infers_zh_primary() {
        let profile = infer_from_model_id("tongyi-qwen-max");
        assert_eq!(profile.primary_lang, Language::ZhCN);
        assert_eq!(profile.source, ProfileSource::NamingHeuristic);
    }

    #[test]
    fn gpt_infers_en_primary() {
        let profile = infer_from_model_id("openai-gpt-5");
        assert_eq!(profile.primary_lang, Language::En);
    }
}
