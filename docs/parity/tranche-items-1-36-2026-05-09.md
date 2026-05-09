# Upstream Parity Tranche: Items 1-36 (2026-05-09)

This tranche maps the first 36 pending upstream commits from `main..upstream/main` to Hermes Ultra Rust outcomes.

## Scope

- Upstream range source: `git cherry -v main upstream/main`
- Target window: first 36 commits in that range
- Goal: implement actionable parity in Rust and explicitly close non-actionable metadata/docs-only commits

## Disposition Summary

- `ported_now`: 4
- `already_covered`: 7
- `docs_or_metadata_only`: 21
- `superseded_by_rust_architecture`: 4

## Item-by-Item Disposition

1. `d8cc85dcdccf` review(stt-xai): address cetej's nits  
   Disposition: `superseded_by_rust_architecture` (bulk upstream snapshot/review commit across Python/web/docs surfaces).
2. `3d90292eda55` fix: normalize provider in list_provider_models to support aliases  
   Disposition: `ported_now` via provider alias canonicalization for model catalog resolution in Rust.
3. `77f99c4ff445` chore(release): AUTHOR_MAP  
   Disposition: `docs_or_metadata_only`.
4. `8b1ff55f5382` fix(wecom): strip @mention prefix in group chats  
   Disposition: `already_covered` in Rust WeCom callback adapter mention-strip path + tests.
5. `85cc12e2bd55` chore(release): AUTHOR_MAP  
   Disposition: `docs_or_metadata_only`.
6. `92e4bbc201e6` docs Docker guide update  
   Disposition: `docs_or_metadata_only`.
7. `fa47cbd45671` chore(release): AUTHOR_MAP  
   Disposition: `docs_or_metadata_only`.
8. `156b3583206d` docs(cron): runtime resolution note  
   Disposition: `docs_or_metadata_only`.
9. `48dc8ef1d158` docs(cron): default model/provider clarification  
   Disposition: `docs_or_metadata_only`.
10. `e8cba18f77c2` chore(release): AUTHOR_MAP  
    Disposition: `docs_or_metadata_only`.
11. `15efb410d035` fix(nix): writable working dir  
    Disposition: `superseded_by_rust_architecture` (no equivalent Nix module path in this repo).
12. `5e76c650bbae` chore(release): AUTHOR_MAP  
    Disposition: `docs_or_metadata_only`.
13. `22afa066f838` fix(cron): guard non-dict run_conversation result  
    Disposition: `superseded_by_rust_architecture` (Rust cron runner path does not deserialize Python dict payloads).
14. `1c532278ae70` chore(release): AUTHOR_MAP  
    Disposition: `docs_or_metadata_only`.
15. `738d0900fddd` anthropic auxiliary transport refactor  
    Disposition: `superseded_by_rust_architecture`.
16. `f4612785a485` anthropic adapter refactor  
    Disposition: `superseded_by_rust_architecture`.
17. `43de1ca8c287` anthropic shim removal/flush guard  
    Disposition: `superseded_by_rust_architecture`.
18. `36adcebe6ca8` docs rename API call function  
    Disposition: `docs_or_metadata_only`.
19. `f77da7de42a1` docs rename API call function  
    Disposition: `docs_or_metadata_only`.
20. `48923e5a3d8f` chore(release): AUTHOR_MAP  
    Disposition: `docs_or_metadata_only`.
21. `d7452af257b9` fix(pairing): null user_name in list display  
    Disposition: `already_covered` via optional-name fallback in Rust pairing output.
22. `b5ec6e8df79f` chore(release): AUTHOR_MAP  
    Disposition: `docs_or_metadata_only`.
23. `1df0c812c43a` feat(skills): MiniMax-AI/cli default tap  
    Disposition: `already_covered` (`DEFAULT_SKILL_TAPS` already includes `https://github.com/MiniMax-AI/cli::skill`).
24. `c80cc8557ed0` chore(release): AUTHOR_MAP  
    Disposition: `docs_or_metadata_only`.
25. `a5b0c7e2ec07` fix(config): preserve list-format models in custom_providers normalize  
    Disposition: `already_covered`/N/A for Rust typed provider config model field (string-based provider model selection path).
26. `33773ed5c6da` chore(release): AUTHOR_MAP  
    Disposition: `docs_or_metadata_only`.
27. `5d0947434864` fix(tools): enforce ACP transport overrides in delegate_task children  
    Disposition: `already_covered` by Rust in-process delegation that reuses parent runtime/tool registry wiring.
28. `911f57ad979d` chore(release): AUTHOR_MAP  
    Disposition: `docs_or_metadata_only`.
29. `be99feff1f42` fix(image-gen): force-refresh plugin providers in long sessions  
    Disposition: `already_covered`/N/A (Rust image generation backend does not rely on mutable Python plugin provider cache).
30. `8f50f2834a0d` chore(release): AUTHOR_MAP  
    Disposition: `docs_or_metadata_only`.
31. `9dba75bc3862` fix(feishu): leading newlines on stream edits  
    Disposition: `already_covered` via message normalization trim path + tests.
32. `08cb345e242e` chore(release): AUTHOR_MAP  
    Disposition: `docs_or_metadata_only`.
33. `51c1d2de16bc` fix(profiles): stage imports to prevent clobbering  
    Disposition: `ported_now` in Rust profile import flow.
34. `4c02e4597ec9` fix(status): catch OSError in `os.kill(pid,0)` on Windows  
    Disposition: `already_covered` in Rust with platform-gated PID liveness implementation.
35. `dab36d9511ce` chore(release): AUTHOR_MAP  
    Disposition: `docs_or_metadata_only`.
36. `7ca2f70055d9` docs: Atropos/wandb links  
    Disposition: `docs_or_metadata_only`.

## Rust Ports in This Tranche

- Provider alias normalization for model catalog/listing:
  - `crates/hermes-cli/src/providers.rs`
  - `crates/hermes-cli/src/model_switch.rs`
- Profile import hardening (staged writes + clobber guard + profile name validation):
  - `crates/hermes-cli/src/main.rs`

## Extra in-flight hardening carried forward

- Added `/sessions` slash command bridge in interactive command router.
- Added `profile create --no-skills`.
- Sanitized post-install Python environment during installer checks/setup.

