# hermes_state.py parity (Rust)

Rust implementation: `crates/hermes-agent/src/session_persistence/` (`SessionPersistence`).

Shared read/search/billing/topic helpers: `crates/hermes-tools/src/state_db/` (`StateDb`).

Python reference: `docs/hermes_state.py` (`SessionDB`).

Database: `$HERMES_HOME/state.db` (legacy `sessions.db` still read if `state.db` absent).

## Implemented

| Area | Notes |
|------|--------|
| Schema v14 tables | `sessions`, `messages`, `state_meta`, `compression_locks`, `schema_version` |
| FTS5 + trigram | Python-style triggers; migrates away from legacy external-content FTS |
| WAL + NFS fallback | `apply_wal_with_fallback`, `format_session_db_unavailable` |
| Write retry | `BEGIN IMMEDIATE` + jitter (15×, 20–150ms) |
| Session CRUD | create/ensure, persist incremental, replace, load, end/reopen |
| Compression lineage | `parent_session_id`, continuation session, compression locks, `get_compression_tip` |
| Titles | sanitize, set/get, unique index, resolve by title |
| Resume | `resolve_resume_session_id`, CLI `/resume` prefers `state.db` |
| Listing | `list_sessions_rich` + compression-tip projection + recursive CTE `order_by_last_active` |
| Search | `StateDb::search_messages` (FTS5, trigram CJK, short-CJK LIKE, context, sort) |
| `session_search` tool | Uses `StateDb` (no duplicate FTS SQL in tool backend) |
| Anchored history | `get_messages_around`, `get_anchored_view`; CLI `/history` prefers DB |
| Token/billing | `TokenCountUpdate` with increment/absolute + cost fields; wired in `run_conversation` |
| Telegram topic mode | DB APIs + Gateway `/topic`, lobby gate, thread session keys, auto-bind, thread-aware replies |
| Prune | ended sessions only, orphan children, Python `started_at` cutoff |
| Gateway index | `gateway_session_index` (Rust extension, not in upstream Python schema) |

## Remaining intentional gaps

| Feature | Reason |
|---------|--------|
| Multimodal content JSON encode on write | Read-side decode for previews/search; write path still plain strings |
| Telegram `getMe` capability check on `/topic` enable | Gateway enables mode locally; Bot API capability probe deferred |
| Session restore via `/topic <id>` | Binding persisted + gateway hydrates bound session from `state.db` on next message |
| Cross-process Python `state.db` binary interchange | Same schema goal; Rust may migrate legacy `sessions.db` in place |
| JSON snapshot `/save` removal | JSON checkpoints kept as fallback/export; `/resume` prefers SQLite |

## Verification

```bash
cargo build -p hermes-agent
cargo build -p hermes-tools
cargo build -p hermes-gateway
cargo build -p hermes-cli
cargo test -p hermes-agent --lib session_persistence
cargo test -p hermes-tools state_db
```
