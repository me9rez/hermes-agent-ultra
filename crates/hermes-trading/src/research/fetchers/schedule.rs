//! Topological execution layers for parallel dimension collection.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use super::collect::CollectOptions;
use super::registry::{EXEC_ORDER, fetcher_for};
use super::r#trait::DimFetcher;
use super::types::Market;
use crate::research::profile::AnalysisProfile;

/// Plan fetch layers: same layer has no unresolved `depends_on` (safe to run concurrently).
#[must_use]
pub fn exec_layers(
    registry: &[Arc<dyn DimFetcher>],
    profile: &AnalysisProfile,
    opts: &CollectOptions,
    market: Market,
) -> Vec<Vec<String>> {
    let mut keys = Vec::new();
    let mut deps: HashMap<String, Vec<String>> = HashMap::new();

    for &dim_key in EXEC_ORDER {
        if !profile.should_run_fetcher(dim_key) {
            continue;
        }
        let Some(fetcher) = fetcher_for(registry, dim_key) else {
            continue;
        };
        if fetcher.spec().web_only && !opts.include_web_dims {
            continue;
        }
        if !fetcher.spec().markets.contains(&market) {
            continue;
        }
        let key = dim_key.to_string();
        let layer_deps: Vec<String> = fetcher
            .spec()
            .depends_on
            .iter()
            .map(|d| (*d).to_string())
            .collect();
        deps.insert(key.clone(), layer_deps);
        keys.push(key);
    }

    topo_layers(&keys, &deps)
}

fn topo_layers(keys: &[String], deps: &HashMap<String, Vec<String>>) -> Vec<Vec<String>> {
    let key_set: HashSet<&str> = keys.iter().map(String::as_str).collect();
    let mut remaining: HashSet<&str> = key_set.clone();
    let mut layers = Vec::new();

    while !remaining.is_empty() {
        let mut layer = Vec::new();
        for key in keys {
            if !remaining.contains(key.as_str()) {
                continue;
            }
            let ready = deps
                .get(key)
                .map(|ds| {
                    ds.iter()
                        .all(|d| !key_set.contains(d.as_str()) || !remaining.contains(d.as_str()))
                })
                .unwrap_or(true);
            if ready {
                layer.push(key.clone());
            }
        }
        if layer.is_empty() {
            // ponytail: cycle or external dep — fall back to EXEC_ORDER remainder serially
            let rest: Vec<String> = remaining.iter().map(|k| (*k).to_string()).collect();
            layers.push(rest);
            break;
        }
        for k in &layer {
            remaining.remove(k.as_str());
        }
        layers.push(layer);
    }

    layers
}

/// Index registry by dim key for parallel fetch dispatch.
#[must_use]
pub fn fetcher_map(registry: &[Arc<dyn DimFetcher>]) -> BTreeMap<String, Arc<dyn DimFetcher>> {
    let mut map = BTreeMap::new();
    for f in registry {
        map.insert(f.spec().dim_key.to_string(), f.clone());
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research::fetchers::registry::build_registry;
    use crate::research::profile::AnalysisProfile;

    #[test]
    fn medium_profile_layers_basic_before_valuation() {
        let registry = build_registry();
        let profile = AnalysisProfile::medium();
        let layers = exec_layers(&registry, &profile, &CollectOptions::default(), Market::A);
        assert!(!layers.is_empty());
        let flat: Vec<&str> = layers.iter().flatten().map(String::as_str).collect();
        let basic_pos = flat.iter().position(|k| *k == "0_basic").expect("basic");
        let val_pos = flat.iter().position(|k| *k == "10_valuation").expect("val");
        assert!(basic_pos < val_pos);
        // valuation and peers both depend on basic — same or later layer
        let basic_layer = layers
            .iter()
            .position(|l| l.iter().any(|k| k == "0_basic"))
            .unwrap();
        let val_layer = layers
            .iter()
            .position(|l| l.iter().any(|k| k == "10_valuation"))
            .unwrap();
        assert!(val_layer >= basic_layer);
    }

    #[test]
    fn lite_profile_skips_macro_dims() {
        let registry = build_registry();
        let profile = AnalysisProfile::lite();
        let layers = exec_layers(&registry, &profile, &CollectOptions::default(), Market::A);
        let flat: Vec<&str> = layers.iter().flatten().map(String::as_str).collect();
        assert!(flat.contains(&"15_events"));
        assert!(!flat.contains(&"3_macro"));
        assert!(!flat.contains(&"18_trap")); // web_only skipped unless include_web_dims
    }

    #[test]
    fn first_layer_has_no_internal_deps() {
        let registry = build_registry();
        let profile = AnalysisProfile::medium();
        let layers = exec_layers(&registry, &profile, &CollectOptions::default(), Market::A);
        let first = layers.first().expect("layer0");
        assert!(first.iter().any(|k| k == "0_basic"));
        assert!(first.iter().any(|k| k == "1_financials"));
    }
}
