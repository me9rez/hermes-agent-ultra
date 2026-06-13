//! Skill installation infrastructure — pure data-processing functions
//! extracted from mod.rs to keep slash command dispatch separate.
//!
//! This module has NO dependency on `App`, `CommandResult`, or slash-command
//! dispatch. It is used by `skills.rs` (CLI skills subcommand) and by unit
//! tests in the parent module.

mod bootstrap;
mod claude_marketplace;
mod clawhub;
mod constants;
mod fallback;
mod github;
mod hub_state;
mod install;
mod lobehub;
mod official;
mod parse;
mod registry;
mod skills_sh;
mod taps;
mod types;

pub(crate) use bootstrap::maybe_run_skill_bootstrap;
pub(crate) use claude_marketplace::resolve_claude_marketplace_skill;
pub(crate) use clawhub::fetch_clawhub_skill_files;
pub(crate) use constants::{
    DEFAULT_SKILL_TAPS, SENTRUX_MCP_ARG, SENTRUX_MCP_COMMAND, SENTRUX_MCP_SERVER_NAME,
};
pub(crate) use fallback::resolve_install_via_fallback_router;
pub(crate) use github::{fetch_skill_files_from_github, github_default_branch};
pub(crate) use hub_state::{
    hash_installed_skill_dir, hash_skill_bundle, read_skills_hub_lock,
    record_skill_install_in_hub_lock, record_skill_uninstall_in_hub_lock, skills_hub_lock_path,
    skills_install_force,
};
pub(crate) use install::{fetch_bundle_for_lock_entry, install_skill_files};
pub(crate) use lobehub::fetch_lobehub_skill_files;
pub(crate) use official::resolve_official_skill_source;
pub(crate) use parse::{
    looks_like_github_repo_slug, parse_explicit_github_skill, parse_registry_prefixed_skill,
    parse_skill_name_and_version, sanitize_skill_install_name,
};
pub(crate) use registry::{
    default_trust_level_for_source, resolve_skill_via_registry_index, search_multi_registry,
};
pub(crate) use skills_sh::{resolve_skills_sh_source, search_skills_sh_registry};
pub(crate) use taps::{
    effective_skill_taps, read_skill_taps, resolve_skill_in_repo, search_skills_via_taps,
    write_skill_taps,
};
pub(crate) use types::{
    InstallFallbackSource, RegistryInstallSource, ResolvedSkillSource, SkillHubInstalledEntry,
    SkillInstallProvenance,
};

#[cfg(test)]
pub(crate) use bootstrap::{
    collect_bootstrap_commands_from_value, execute_bootstrap_command,
    is_allowed_bootstrap_executable, parse_bootstrap_command, parse_skill_bootstrap_plan,
    prompt_bootstrap_yes_no, push_bootstrap_command_if_present, skill_auto_bootstrap_enabled,
    skill_bootstrap_force_confirmed,
};
#[cfg(test)]
pub(crate) use clawhub::{detect_archive_format, extract_clawhub_archive};
#[cfg(test)]
pub(crate) use github::{github_repo_tree, github_request};
#[cfg(test)]
pub(crate) use hub_state::{
    append_skills_hub_audit, collect_skill_files_recursive, now_rfc3339,
    skill_guard_enforce_bundle, skills_hub_audit_path, skills_hub_state_dir, write_skills_hub_lock,
};
#[cfg(test)]
pub(crate) use official::{canonicalize_official_skill_dir, official_skill_path_candidates};
#[cfg(test)]
pub(crate) use parse::{
    canonicalize_skills_sh_identifier, ensure_safe_relative_path, parse_repo_skill_identifier,
    parse_skill_tap_spec,
};
#[cfg(test)]
pub(crate) use registry::{
    build_lobehub_skill_markdown, fetch_hermes_skills_index, resolved_source_from_index,
    score_registry_match, skill_source_priority, sort_registry_skill_records,
};
#[cfg(test)]
pub(crate) use taps::{
    merged_skill_taps, normalize_tap_path_for_storage, read_skill_subscriptions,
    resolve_skill_via_taps, subscription_entry_to_source, subscription_source_to_tap,
    tap_object_to_string, tap_string_to_object,
};
#[cfg(test)]
pub(crate) use types::{
    GitHubTreeEntry, HermesSkillsIndexEntry, LobeHubAgentResponse, ParsedBootstrapCommand,
    RegistrySkillRecord, SkillBootstrapPlan, SkillTapSpec, SkillsHubLockFile,
};
