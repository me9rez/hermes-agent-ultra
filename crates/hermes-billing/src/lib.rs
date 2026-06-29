//! Billing, quota, and feature gate for Terra.

pub mod consent_gate;
pub mod feature_gate;
pub mod heuristic;
pub mod lang_profile;
pub mod quota;
pub mod tier_mapping;
pub mod tool_budget;
pub mod tool_budget_engine;

pub use consent_gate::{ConsentGate, ConsentGateError};
pub use feature_gate::{
    FeatureGate, FeatureGateError, VerticalCap, check_model_access, effective_provider_tier,
    resolve_provider_tier,
};
pub use hermes_accounts::ProviderTier;
pub use heuristic::infer_from_model_id;
pub use lang_profile::{
    AutoBlendContext, Language, ModelLanguageProfile, ProfileSource, default_profile,
    is_low_confidence, resolve_profile, telemetry_eligible,
};
pub use quota::{QuotaEngine, QuotaError};
pub use tier_mapping::{
    GlobalTierMapping, VerticalTierOverrides, default_global_mappings, default_vertical_overrides,
    mapping_requires_tier, resolve_model, tier_at_least,
};
pub use tool_budget::{ToolBudget, ToolId, default_tool_budgets};
pub use tool_budget_engine::{BudgetError, ToolBudgetEngine};
