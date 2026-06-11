#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SkillsExecutionTier {
    Trusted,
    Balanced,
    Open,
}

impl SkillsExecutionTier {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "trusted" => Some(Self::Trusted),
            "balanced" => Some(Self::Balanced),
            "open" | "permissive" => Some(Self::Open),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Trusted => "trusted",
            Self::Balanced => "balanced",
            Self::Open => "open",
        }
    }
}

pub(crate) fn skills_execution_tier() -> SkillsExecutionTier {
    std::env::var("HERMES_SKILLS_EXECUTION_TIER")
        .ok()
        .as_deref()
        .and_then(SkillsExecutionTier::parse)
        .unwrap_or(SkillsExecutionTier::Balanced)
}

pub(crate) fn skills_tier_bypass_enabled() -> bool {
    std::env::var("HERMES_SKILLS_TIER_BYPASS")
        .ok()
        .is_some_and(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

pub(crate) fn skills_action_blocked_by_tier(
    tier: SkillsExecutionTier,
    action: &str,
    name: Option<&str>,
) -> bool {
    let name_lc = name.map(|v| v.to_ascii_lowercase());
    match tier {
        SkillsExecutionTier::Trusted => {
            matches!(
                action,
                "install" | "update" | "publish" | "uninstall" | "reset" | "subscribe"
            ) || (action == "tap" && matches!(name_lc.as_deref(), Some("add" | "remove")))
                || (action == "snapshot" && matches!(name_lc.as_deref(), Some("import")))
        }
        SkillsExecutionTier::Balanced => {
            matches!(action, "publish" | "reset")
                || (action == "snapshot" && matches!(name_lc.as_deref(), Some("import")))
        }
        SkillsExecutionTier::Open => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_action_blocked_by_tier_enforces_expected_matrix() {
        assert!(skills_action_blocked_by_tier(
            SkillsExecutionTier::Trusted,
            "install",
            None
        ));
        assert!(skills_action_blocked_by_tier(
            SkillsExecutionTier::Trusted,
            "tap",
            Some("add")
        ));
        assert!(!skills_action_blocked_by_tier(
            SkillsExecutionTier::Trusted,
            "list",
            None
        ));
        assert!(skills_action_blocked_by_tier(
            SkillsExecutionTier::Balanced,
            "publish",
            None
        ));
        assert!(!skills_action_blocked_by_tier(
            SkillsExecutionTier::Balanced,
            "install",
            None
        ));
        assert!(!skills_action_blocked_by_tier(
            SkillsExecutionTier::Open,
            "publish",
            None
        ));
    }
}
