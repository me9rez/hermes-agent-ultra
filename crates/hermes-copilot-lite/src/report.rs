//! Analysis report types.

use serde::{Deserialize, Serialize};

use hermes_strategies::Decision;

/// Combined analysis report produced by [`CopilotLite::analyze`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisReport {
    /// Symbol that was analyzed.
    pub symbol: String,
    /// Number of OHLCV data points used.
    pub data_points: usize,
    /// Per-strategy decisions: (strategy_name, decisions).
    pub strategy_decisions: Vec<(String, Vec<Decision>)>,
}

impl AnalysisReport {
    /// Returns a summary string suitable for chat display.
    pub fn summary(&self) -> String {
        let mut lines = vec![
            format!("## Analysis: {}", self.symbol),
            format!("Data points: {}", self.data_points),
            String::new(),
        ];
        for (name, decisions) in &self.strategy_decisions {
            let last = decisions.last();
            let signal_str = last
                .map(|d| format!("{:?}", d.signal))
                .unwrap_or_else(|| "N/A".to_string());
            lines.push(format!(
                "- **{name}**: last signal = {signal_str}"
            ));
        }
        lines.join("\n")
    }
}
