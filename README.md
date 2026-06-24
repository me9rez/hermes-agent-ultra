# ⚡ Hermes Flash

**[English](./README.md)** | **[中文](./README_ZH.md)**

```text
  ██   ██ ███████ ██████  ███    ███ ███████ ███████
  ██   ██ ██      ██   ██ ████  ████ ██      ██
  ███████ █████   ██████  ██ ████ ██ █████   ███████
  ██   ██ ██      ██   ██ ██  ██  ██ ██           ██
  ██   ██ ███████ ██   ██ ██      ██ ███████ ███████

            F  L  A  S  H
```

**Blazing-fast, Rust-native autonomous agent runtime.** Built for developers who demand speed, safety, and sovereignty over their AI toolchain. Powered by the **FlowyAIPC** cloud ecosystem for managed models, media generation, and multi-platform delivery.

> **Hermes × FlowyAIPC** — local agent intelligence meets cloud scale.

## ✨ The Elevator Pitch

Hermes Flash is a fully autonomous agent that lives in your terminal. It thinks, codes, searches the web, runs commands, and orchestrates entire multi-step workflows — all at Rust speed, with a gorgeous TUI and zero cloud lock-in.

- **Raw performance**: cold start under 50 ms; streaming tool calls with zero-lag rendering
- **Rust safety**: memory-safe, data-race-free, crash-resistant agent loop
- **Local-first**: works offline with Ollama, llama.cpp, vLLM, MLX, SGLang, TGI, or Apple ANE
- **Cloud-capable**: OpenAI, Anthropic, Nous Portal, FlowyAIPC server, or any OpenAI-compatible endpoint
- **Voice-first**: real-time ASR + LLM + TTS voice dialog pipeline (`hermes talk` — optional feature)
- **Media generation**: AI image & video via Flowy cloud APIs with multi-step workflow orchestration
- **Operator-first**: policy engine, approval gates, incident packs, replay/debugging, and a deep doctor diagnostic surface

## ⚡ Performance — Rust vs Python

Numbers don't lie. Here's how Hermes Flash stacks up against the upstream Python agent running the same workload:

| Metric | 🐍 Python (`hermes-agent`) | ⚡ Hermes Flash (Rust) | Advantage |
|---|---|---|---|
| **Binary size** | 500 MB+ (interpreter + venv + deps) | **57 MB** single file · **17 MB** compressed | **~30× smaller** to ship |
| **Cold start** | 800–1200 ms (Python import chain) | **< 50 ms** (stub) · **~200 ms** (full agent) | **4–6× faster** |
| **Baseline memory** | 180–400 MB (CPython + loaded modules) | **30–80 MB** (single process, no GC) | **~5× less RAM** |
| **Tool execution** | 1× (baseline, GIL-bound) | **2–5× faster** (parallel dispatch, zero-copy JSON) | **CPU saturation** |
| **Streaming latency** | 15–40 ms per chunk (Python async) | **< 5 ms** per chunk (tokio, no GIL) | **3–8× lower jitter** |
| **Runtime deps** | Python 3.11+, pip, venv, 200+ packages | **Zero** — single static binary | **No venv hell** |
| **Concurrency** | 1 thread (GIL) | **N cores** (tokio work-stealing) | **True parallelism** |
| **Crash recovery** | Traceback → restart | Memory-safe, data-race-free, `catch_unwind` at loop boundary | **Resilient** |

> **TL;DR** — Hermes Flash runs the same agent loop in a fraction of the memory, starts instantly, handles concurrent requests natively, and ships as a single file you can `scp` anywhere.

---

## 🧬 Our DNA — Honoring the Projects That Made Us

Hermes Flash didn't start in a vacuum. We stand on the shoulders of three projects that shaped every design decision we made:

| Project | Role | What We Learned |
|---|---|---|
| **[NousResearch/hermes-agent](https://github.com/NousResearch/hermes-agent)** | **Canonical upstream** | The original Python agent. Defined the product surface, conversation loop, tool-format conventions, provider profiles, and the entire operator workflow. We track it continuously for functional parity. |
| **[Lumio-Research/hermes-agent-rs](https://github.com/Lumio-Research/hermes-agent-rs)** | **Rust foundation** | The first Rust re-implementation. Gave us the core workspace layout, TUI skeleton, MCP transport layer, gateway architecture, and proved that a pure-Rust agent loop was viable. |
| **Hermes Agent Ultra** | **The Ultra layer** | Added the policy engine, memory fusion, smart routing, parity-upkeep automation, elite sync gates, quorum agents, trading pipelines, eval harness, and the full operator control plane that defines Flash's differentiated surface. |

> **Hermes Flash** is the culmination of all three lineages: upstream parity from `hermes-agent`, Rust engineering from `hermes-agent-rs`, and the expanded operator/autonomy surface from Ultra — merged into one cohesive, turbocharged runtime.

We reuse substantial code and architecture from both predecessor projects and extend them with purpose. See [UPSTREAM_ATTRIBUTION.md](./UPSTREAM_ATTRIBUTION.md) for full provenance.

---

## 🚀 What's Inside

### ⚙️ Core Runtime
- **Fully autonomous agent loop**: nested reasoning, tool-call → execute → observe → continue, with configurable turn budgets
- **Chat + TUI**: interactive terminal UI with markdown rendering, syntax highlighting, and inline streaming
- **One-shot mode**: `hermes flash --query "refactor this crate"` for CI/CD and scripting
- **Gateway server**: long-running daemon powering Telegram, Discord, WeChat, Slack, and custom webhook platforms simultaneously
- **Session persistence**: save, resume, fork, and time-travel through conversation checkpoints

### 🧠 Intelligence Layer
- **Smart model router**: auto-selects the best model per task based on capability requirements, context size, cost, and provider health
- **Swarm orchestration**: `hermes flash swarm run 4 sequential` — execute tasks across multiple models in parallel or series
- **Quorum agents**: run the same query through N models and synthesize a consensus answer with confidence scoring
- **Error classifier**: automatic retry strategies by error category (rate-limit, context-overflow, auth, transient)
- **Context Engine**: ContextLattice + external memory providers with scored fusion, dedup, and noise reduction

### 🎨 Media & Generation
- **Flowy image generation**: AI image creation via cloud APIs with rich prompt guidance
- **Flowy video generation**: text-to-video and image-to-video with async poll-based delivery
- **Media workflows**: multi-step pipeline (`media_workflow_plan` → `media_workflow_run` → `media_workflow_status`) with built-in templates for txt2img, img2video, storyboard-to-video, and more
- **Gateway progress UX**: live progress messages during media generation tasks (`正在生成图片…`, `云端正在渲染视频…`)
- **Model catalog**: interactive model picker (`hermes media models pick image|video`) from Flowy server

### 🎤 Voice Dialog — `hermes talk`
- **Real-time ASR + LLM + TTS**: full duplex voice conversation pipeline (requires `--features talk`)
- **Device management**: `list-devices`, `probe-capture`, `probe-playback` for audio hardware setup
- **Speaker enrollment**: voiceprint registration for personalized wake-word detection
- **Channel mode**: AIPC talk channel integration for multi-modal agent interaction
- **Cross-compilation**: `talk-rockchip` feature for ARM/Rockchip edge devices

### 🔧 Tool System
- **40+ built-in tools**: web search, file ops, terminal, browser, code execution, git, media extraction, image/video generation, and more
- **MCP (Model Context Protocol)**: full client/server with stdio + HTTP/SSE transport, OAuth, and sandbox profiles (`strict`, `balanced`, `relaxed`)
- **Policy engine**: per-tool allow/deny/regex-match rules with `enforce`, `audit`, or `simulate` modes
- **Approval system**: dangerous-command detection, user-in-the-loop confirmation for risky terminal ops
- **Tool preview/simulate**: dry-run a tool call to see what it *would* do before letting it happen

### 🛡️ Security & Control
- **Skill guard**: mandatory security scan on install and before execution; `strict` / `relaxed` / `off` modes
- **Policy presets**: `strict`, `balanced`, `dev`, `relaxed` — apply to tools, skills, and MCP servers
- **Runtime overrides**: every guard can be relaxed or tightened via env vars
- **Sensitive-field redaction**: traces, logs, and replay artifacts automatically scrub API keys, tokens, and PII
- **Provenance verification**: cryptographic signing and verification of skills, configs, and parity artifacts

### 📊 Operator Dashboard
- **`hermes flash doctor --deep --snapshot --bundle`**: full system diagnostic with exportable report
- **`hermes flash server`**: FlowyAIPC remote LLM server account management (config, login via WeChat/email, whoami)
- **`hermes flash media`**: interactive image/video model picker, config, and workflow template browser
- **`/raw trace`**: inspect unwrapped LLM payloads, tool arguments, and MCP messages for debugging
- **`/timetravel`**: jump to any checkpoint in a session and replay from there
- **`/policy`**: inspect, switch, and hot-reload policy packs at runtime
- **`/qos`**: view provider route health, latency histograms, and autotune recommendations

### 🏪 Skills Ecosystem
- **Multi-registry**: `official/`, `skills.sh/`, `github/`, `lobehub/`, `clawhub/`, `claude-marketplace/`
- **Curator**: autonomous classification, pruning, and consolidation of your skill collection
- **Bundled skills**: 25+ categories shipped with the binary — from `devops` to `finance` to `red-teaming`
- **Local taps**: install, develop, and share skills from your filesystem

### 📈 Trading & Finance *(optional module)*
- **Live market data**: multi-provider quote aggregation with caching
- **Technical indicators**: SMA, EMA, RSI, MACD, Bollinger Bands, and more
- **Backtesting engine**: replay strategies against historical data
- **Equity research**: automated gate analysis, SEC filing parsing, sentiment scoring

### 🔄 Parity Upkeep
- **Continuous upstream sync**: fetch `NousResearch/hermes-agent`, generate drift queues, apply controlled roll-forwards
- **Parity test harness**: Python fixture → Rust output comparison across all active modules
- **Elite sync gate**: differential parity checks with red-team / adversarial gating
- **Autonomous parity PRs**: automated draft PR generation with drift classification and risk labels

---

## 📦 Install

### One-liner

```bash
curl -fsSL https://raw.githubusercontent.com/sheawinkler/hermes-agent-ultra/main/scripts/install.sh | bash
```

Installs `hermes-flash` only. Existing `hermes` or `hermes-ultra` installs are untouched.

### From source

```bash
cargo install --git https://github.com/sheawinkler/hermes-agent-ultra hermes-cli --locked --bin hermes-flash
```

### With Nous Portal (managed model access + tool backends)

```bash
curl -fsSL https://raw.githubusercontent.com/sheawinkler/hermes-agent-ultra/main/scripts/install.sh | bash -s -- --setup --portal
```

---

## ⚡ Quick Start

```bash
# Setup wizard — configure providers, tools, and preferences
hermes-flash setup

# Start an interactive session
hermes-flash

# One-shot query
hermes-flash --query "explain this codebase"

# Run as a multi-platform gateway
hermes-flash gateway --live

# Diagnostics snapshot
hermes-flash doctor --deep --snapshot --bundle

# --- FlowyAIPC cloud features ---

# Configure & log into remote LLM server
hermes-flash server config init
hermes-flash server login

# Browse & pick image/video generation models
hermes-flash media init
hermes-flash media models pick image

# --- Voice dialog (requires --features talk at build) ---

# Initialize voice config and test audio devices
hermes-flash talk init
hermes-flash talk list-devices

# Start real-time voice conversation
hermes-flash talk run
```

---

## 🏠 Local & Self-Hosted Backends

Hermes Flash runs with zero API keys when you bring your own model:

| Backend | Default Endpoint | Env Override |
|---|---|---|
| Ollama | `http://127.0.0.1:11434/v1` | `OLLAMA_BASE_URL` |
| llama.cpp | `http://127.0.0.1:8080/v1` | `LLAMA_CPP_BASE_URL` |
| vLLM | `http://127.0.0.1:8000/v1` | `VLLM_BASE_URL` |
| MLX | `http://127.0.0.1:8080/v1` | `MLX_BASE_URL` |
| Apple ANE | `http://127.0.0.1:8081/v1` | `APPLE_ANE_BASE_URL` |
| SGLang | `http://127.0.0.1:30000/v1` | `SGLANG_BASE_URL` |
| TGI | `http://127.0.0.1:8082/v1` | `TGI_BASE_URL` |

Full guide: [docs/local-backends.md](./docs/local-backends.md)

---

## 🎛️ Operator Commands

### In-session slash commands
```
/model explain                    # Why was this model chosen?
/model why-not --cap reasoning    # What's missing from current model?
/swarm plan graph                 # Visualize swarm execution DAG
/swarm run 4 sequential           # Run 4-model swarm
/quorum ask "..."                 # Consensus across models

/policy list                      # Active policy packs
/policy strict                    # Lock down all tools
/policy balanced                  # Default safety profile

/raw trace status                 # Inspect raw provider payloads
/raw trace export 200             # Export last 200 turns

/timetravel list                  # Available checkpoints
/timetravel goto <snapshot>       # Rewind and branch

/qos status                       # Provider health dashboard
/qos health                       # Latency + error rate summary

/ops autopilot status             # Intelligence-performance autopilot
/ops autopilot recommend          # Tuning suggestions
/ops budget balanced              # Set intelligence budget
```

### CLI subcommands
```
hermes-flash server config        # FlowyAIPC remote server settings
hermes-flash server login          # Authenticate via WeChat or email
hermes-flash server whoami         # Show login status

hermes-flash media                 # Image/video generation settings
hermes-flash media models          # List cloud media models
hermes-flash media workflows       # Browse workflow templates

hermes-flash talk init             # Set up voice dialog config
hermes-flash talk run              # Start real-time voice session
hermes-flash talk list-devices     # List audio capture/playback devices
hermes-flash talk enroll           # Register voiceprint
```

---

## 📐 Architecture

```
┌─────────────────────────────────────────────┐
│  hermes-cli   ←  TUI · CLI · Setup · Ops    │
├─────────────────────────────────────────────┤
│  hermes-agent ←  Agent Loop · Memory ·      │
│                  Routing · Context · Replay  │
├─────────────────────────────────────────────┤
│  hermes-intelligence                        │
│  ┌──────────┬──────────┬─────────────┐      │
│  │ Router   │ Swarm    │ Error Class │      │
│  │ Quorum   │ Insights │ Prompt      │      │
│  └──────────┴──────────┴─────────────┘      │
├─────────────────────────────────────────────┤
│  hermes-tools  ←  Registry · Policy ·       │
│                   Dispatch · Approval        │
│  hermes-mcp    ←  Client · Server · Sandbox │
│  hermes-skills ←  Store · Guard · Curator   │
├─────────────────────────────────────────────┤
│  hermes-gateway ←  Platforms · Sessions ·   │
│                     Delivery · SSRF          │
├─────────────────────────────────────────────┤
│  hermes-core   ←  Types · Traits · Time ·   │
│                   Errors · Schema            │
│  hermes-config ←  Config model & loading    │
│  hermes-telemetry ← Tracing · Metrics       │
│  hermes-cron   ←  Scheduled tasks           │
│  hermes-eval   ←  Evaluation harness        │
│  hermes-trading  ←  Quotes · Indicators ·   │
│                     Backtest · Research      │
│  hermes-talk     ←  ASR · TTS · Voice       │
│                     Dialog (optional)        │
│  hermes-media-   ←  Image · Video · Flowy   │
│    workflows        Workflow Orchestration   │
│  hermes-server-  ←  FlowyAIPC Client ·      │
│    client           Auth · Device Activation │
```

---

## 🔐 Security Posture

- **Skill scanning**: blocks dangerous patterns (destructive ops, restricted URLs, shell injection) before install and before every execution
- **Policy enforcement**: tool execution gated through centralized policy engine with structured deny/audit/simulate decisions
- **MCP sandboxing**: `strict` profile strips sensitive env vars, enforces command allowlists, caps message sizes
- **Redaction**: API keys, tokens, and PII automatically scrubbed from traces, logs, and incident packs
- **Runtime overrides**:
  - `HERMES_SKILL_GUARD_MODE=relaxed`
  - `HERMES_TOOL_POLICY_PRESET=relaxed`
  - `HERMES_MAX_TURNS_UNLIMITED=1`

---

## 🤝 Contributing

Read [CONTRIBUTING.md](./CONTRIBUTING.md) for setup, PR expectations, the single-module PR rule, and the parity completeness gate.

---

## 📜 License

This repository's original contributions are MIT-licensed. See [LICENSE](./LICENSE), [NOTICE](./NOTICE), and [UPSTREAM_ATTRIBUTION.md](./UPSTREAM_ATTRIBUTION.md) for full provenance and upstream attribution.

---

<p align="center">
  <sub>Built with Rust · ratatui · tokio · sherpa-onnx · cpal · and the audacity to ship</sub><br/>
  <sub>⚡ Hermes Flash — Speed of Thought  |  Hermes × FlowyAIPC</sub>
</p>
