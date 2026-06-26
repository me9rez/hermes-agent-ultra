//! Equity research: valuation models, scoring, persona panel (0py, no Python).

pub mod analyze;
pub mod confidence_supplement;
pub mod fetchers;
pub mod gate;
pub mod models;
pub mod personas;
pub mod profile;
pub mod report;
pub mod report_filter;
pub mod scoring;
pub mod synthesis;
pub mod types;

pub use analyze::{analyze_stock, snapshot_from_inputs};
pub use confidence_supplement::supplement_snapshot_confidence;
pub use fetchers::{CollectOptions, CollectOutput, collect_dims, enrich_snapshot};
pub use gate::{g1_hard_dim_ratio, g1_passes, g2_passes};
pub use profile::{AnalysisDepth, AnalysisProfile};
pub use synthesis::{
    ReportPaths, SynthesisFormatOutput, SynthesisReport, build_synthesis,
    build_synthesis_format_output, build_synthesis_parts,
};
pub use types::{
    DataConfidence, DcfAssumptions, FeatureVector, FundamentalsSnapshot, ProvenanceSource,
};
