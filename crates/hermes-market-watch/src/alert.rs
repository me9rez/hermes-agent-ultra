//! Alert engine: condition evaluation and trigger management.

use serde::{Deserialize, Serialize};

use crate::error::WatchError;
use crate::quote::Quote;

/// A condition that can be evaluated against a [`Quote`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AlertCondition {
    /// Triggers when price crosses above `threshold`.
    PriceAbove { threshold: f64 },
    /// Triggers when price crosses below `threshold`.
    PriceBelow { threshold: f64 },
    /// Triggers when absolute change percentage exceeds `pct`.
    ChangePctExceeds { pct: f64 },
    /// Triggers when volume exceeds `threshold`.
    VolumeAbove { threshold: f64 },
}

/// An alert definition binding a symbol to a condition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    pub id: String,
    pub symbol: String,
    pub condition: AlertCondition,
    #[serde(default)]
    pub enabled: bool,
}

/// Result of evaluating an alert against a quote.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertTrigger {
    pub alert_id: String,
    pub symbol: String,
    pub message: String,
}

/// Engine that evaluates alert conditions against incoming quotes.
#[derive(Debug, Default)]
pub struct AlertEngine {
    alerts: Vec<Alert>,
}

impl AlertEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an alert.
    pub fn add_alert(&mut self, alert: Alert) {
        self.alerts.push(alert);
    }

    /// Remove an alert by id.
    pub fn remove_alert(&mut self, alert_id: &str) -> Result<(), WatchError> {
        let before = self.alerts.len();
        self.alerts.retain(|a| a.id != alert_id);
        if self.alerts.len() == before {
            return Err(WatchError::InvalidCondition(format!(
                "alert id not found: {alert_id}"
            )));
        }
        Ok(())
    }

    /// Evaluate all enabled alerts against the given quote, returning any
    /// triggers.
    pub fn evaluate(&self, quote: &Quote) -> Vec<AlertTrigger> {
        self.alerts
            .iter()
            .filter(|a| a.enabled && a.symbol == quote.symbol)
            .filter_map(|alert| {
                let triggered = match &alert.condition {
                    AlertCondition::PriceAbove { threshold } => quote.price >= *threshold,
                    AlertCondition::PriceBelow { threshold } => quote.price <= *threshold,
                    AlertCondition::ChangePctExceeds { pct } => {
                        quote.change_pct.abs() >= *pct
                    }
                    AlertCondition::VolumeAbove { threshold } => quote.volume >= *threshold,
                };
                if triggered {
                    Some(AlertTrigger {
                        alert_id: alert.id.clone(),
                        symbol: quote.symbol.clone(),
                        message: format!(
                            "{} triggered for {} @ {}",
                            alert.condition.description(),
                            quote.symbol,
                            quote.price
                        ),
                    })
                } else {
                    None
                }
            })
            .collect()
    }
}

impl AlertCondition {
    fn description(&self) -> String {
        match self {
            Self::PriceAbove { threshold } => format!("price_above({threshold})"),
            Self::PriceBelow { threshold } => format!("price_below({threshold})"),
            Self::ChangePctExceeds { pct } => format!("change_pct_exceeds({pct}%)"),
            Self::VolumeAbove { threshold } => format!("volume_above({threshold})"),
        }
    }
}
