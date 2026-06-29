//! Analysis depth profiles (UZI `analysis_profile.py` lite/medium subset).

use std::collections::HashSet;

use crate::research::fetchers::dim_keys;

/// Thinking depth tier (deep reserved for future `/ic-memo`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisDepth {
    Lite,
    Medium,
}

/// Top-10 investors for `/quick-scan` (UZI quick-scan.md).
pub const LITE_INVESTOR_IDS: &[&str] = &[
    "buffett",
    "duan",
    "lynch",
    "oneill",
    "wood",
    "marks",
    "minervini",
    "zhang_mz",
    "zhao_lg",
    "chen_xq",
];

/// Scoring dimension keys included in lite mode.
pub const LITE_SCORE_DIM_KEYS: &[&str] = &[
    "1_financials",
    "2_kline",
    "10_valuation",
    "11_governance",
    "15_events",
    "16_lhb",
    "18_trap",
];

/// Runtime profile controlling fetchers, panel size, and model set.
#[derive(Debug, Clone)]
pub struct AnalysisProfile {
    pub depth: AnalysisDepth,
    fetchers_enabled: HashSet<&'static str>,
    pub run_comps_lbo_three_stmt: bool,
    pub allow_web_supplement: bool,
}

impl AnalysisProfile {
    #[must_use]
    pub fn lite() -> Self {
        let mut fetchers_enabled = HashSet::from([
            dim_keys::BASIC,
            dim_keys::FINANCIALS,
            dim_keys::KLINE,
            dim_keys::VALUATION,
            dim_keys::GOVERNANCE,
            dim_keys::EVENTS,
            dim_keys::LHB,
            dim_keys::TRAP,
        ]);
        // ponytail: trap always on for quick-scan per UZI quick-scan.md
        fetchers_enabled.insert(dim_keys::TRAP);

        Self {
            depth: AnalysisDepth::Lite,
            fetchers_enabled,
            run_comps_lbo_three_stmt: false,
            allow_web_supplement: true,
        }
    }

    #[must_use]
    pub fn medium() -> Self {
        Self {
            depth: AnalysisDepth::Medium,
            fetchers_enabled: HashSet::from_iter(dim_keys::ALL.iter().copied()),
            run_comps_lbo_three_stmt: true,
            allow_web_supplement: true,
        }
    }

    #[must_use]
    pub fn from_depth_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "lite" | "quick" | "quick-scan" => Self::lite(),
            _ => Self::medium(),
        }
    }

    #[must_use]
    pub fn is_lite(&self) -> bool {
        self.depth == AnalysisDepth::Lite
    }

    #[must_use]
    pub fn depth_label(&self) -> &'static str {
        match self.depth {
            AnalysisDepth::Lite => "lite",
            AnalysisDepth::Medium => "medium",
        }
    }

    #[must_use]
    pub fn should_run_fetcher(&self, dim_key: &str) -> bool {
        self.fetchers_enabled.contains(dim_key)
    }

    #[must_use]
    pub fn lite_investor_ids(&self) -> Option<&'static [&'static str]> {
        if self.is_lite() {
            Some(LITE_INVESTOR_IDS)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lite_fetcher_set_includes_trap() {
        let p = AnalysisProfile::lite();
        assert!(p.should_run_fetcher(dim_keys::TRAP));
        assert!(p.should_run_fetcher(dim_keys::FINANCIALS));
        assert!(!p.should_run_fetcher(dim_keys::MACRO));
    }

    #[test]
    fn medium_runs_all_registered_dims() {
        let p = AnalysisProfile::medium();
        for key in dim_keys::ALL {
            assert!(p.should_run_fetcher(key), "missing {key}");
        }
    }

    #[test]
    fn from_depth_str_maps_quick_scan() {
        assert!(AnalysisProfile::from_depth_str("quick-scan").is_lite());
        assert!(!AnalysisProfile::from_depth_str("medium").is_lite());
    }
}
