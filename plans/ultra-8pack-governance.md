# Hermes Ultra 8-Pack Governance Plan

Date: 2026-05-10
Branch: `feat/ultra-8pack-governance`

## Scope
Implement and harden items 1-8 with runtime-safe behavior and parity gates.

## Items
1. Optional capability-aware planner router
   - Add `/plan caps [off|advisory|enforce]`.
   - Default to `off` to keep this optional.
   - Gate queued planner tasks against model capabilities only when enabled.

2. ContextLattice session memory compaction governance
   - Extend `/autocompact` with governance modes: `off|advisory|enforce`.
   - On compaction, write ContextLattice checkpoint via orchestration script.
   - In `enforce`, emit lifecycle failure signals if checkpoint write fails.

3. Deterministic trace debugger ergonomics
   - Add `/raw trace focus <trace-id> [N]`.
   - Add `/raw trace graph [N]` for lineage edges.

4. Policy profile trust-tier integration
   - Bind policy profiles to skills trust tier.
   - Surface tier in `/policy list|status|switch`.

5. Skills quality/fallback surface
   - Add `/skills quality` scoring output with fallback recommendations.

6. Provider failover fabric controls
   - Add `/model failover [status|set|clear]`.
   - Persist runtime chain via `HERMES_FALLBACK_MODELS` / `HERMES_FALLBACK_MODEL`.
   - Rebuild active agent immediately after changes.

7. Behavioral parity suite + CI wiring
   - Add `scripts/run-behavioral-parity-suite.py` token contract checks.
   - Wire into `.github/workflows/parity-audit.yml`.
   - Upload generated parity report as artifact.

8. Ops cockpit surface
   - Add `/ops cockpit` summary command.
   - Add TUI lane mode toggle (`Ctrl+O`) for live lane vs ops cockpit.

## Verification
- `cargo fmt --all`
- `cargo test -p hermes-cli build_agent_config_maps_failover`
- `cargo test -p hermes-cli single_failover_model_from_env`
- `cargo test -p hermes-agent call_llm_with_retry_strips_provider_prefix_for_primary_and_fallback_models`
- `cargo test -p hermes-cli -p hermes-agent --no-run`
- `python3 scripts/run-behavioral-parity-suite.py --repo-root .`

## Result
All planned items implemented and validated on this branch before merge.
