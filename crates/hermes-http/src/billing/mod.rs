use axum::Json;
use axum::Router;
use axum::routing::get;
use chrono::Utc;
use hermes_accounts::{QuotaState, Tier};
use hermes_billing::{
    FeatureGate, default_global_mappings, default_vertical_overrides, resolve_provider_tier,
};
use serde::Serialize;

use crate::HttpServerState;

#[derive(Debug, Serialize)]
pub struct TierMappingResponse {
    pub schema_version: u32,
    pub mappings: Vec<hermes_billing::GlobalTierMapping>,
    pub vertical_overrides: Vec<hermes_billing::VerticalTierOverrides>,
}

#[derive(Debug, Serialize)]
pub struct QuotaResponse {
    pub tier: Tier,
    pub quota: QuotaState,
    pub effective_provider_tier: hermes_accounts::ProviderTier,
    pub max_concurrent_tasks: u32,
    pub max_tasks_per_day: Option<u32>,
}

pub async fn tier_mapping_handler() -> Json<TierMappingResponse> {
    Json(TierMappingResponse {
        schema_version: 1,
        mappings: default_global_mappings(),
        vertical_overrides: default_vertical_overrides(),
    })
}

pub async fn quota_handler() -> Json<QuotaResponse> {
    let tier = Tier::Free;
    let gate = FeatureGate::for_tier(tier);
    let account = hermes_accounts::Account {
        user_id: hermes_tasks::UserId::new(),
        email: None,
        oauth_bindings: vec![],
        tier,
        quota: QuotaState {
            period_start: Utc::now(),
            tokens_remaining_input: 100_000,
            tokens_remaining_output: 100_000,
            vertical_caps: std::collections::HashMap::new(),
        },
        provider_prefs: hermes_accounts::ProviderPreferences::default(),
        created_at: Utc::now(),
        last_active_at: Utc::now(),
    };

    Json(QuotaResponse {
        tier,
        quota: account.quota.clone(),
        effective_provider_tier: resolve_provider_tier(&account),
        max_concurrent_tasks: gate.max_concurrent_tasks,
        max_tasks_per_day: gate.max_tasks_per_day,
    })
}

pub fn routes() -> Router<HttpServerState> {
    Router::new()
        .route("/v1/billing/tier-mapping", get(tier_mapping_handler))
        .route("/api/billing/quota", get(quota_handler))
}
