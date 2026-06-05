# 版本管理与 OTA 更新维护指南

## 概述

Hermes Agent Ultra 采用多源（GitHub + ModelScope）、多渠道（stable/beta/rc/nightly）的 OTA 自更新架构。本文档面向日常版本维护工作，涵盖发版流程、渠道管理、manifest 配置、CI 流水线细节及已知限制等操作要点。

> **读者对象**：版本发布工程师、维护者、需要了解发版机制的贡献者。
> **前置要求**：具有仓库 write 权限，本地安装了 `git`、`cargo`，CI secrets（`MODELSCOPE_TOKEN`）已配置。

---

## 一、设计理念

### 1.1 核心原则

- **语义化版本**：所有版本号遵循 SemVer 2.0.0（MAJOR.MINOR.PATCH[-prerelease]），版本比较由 `semver` crate 提供编译时类型安全保证
- **策略与数据分离**：版本比较逻辑通过 `VersionPolicy` trait 抽象，更新元数据（channel/forced/min_version）与策略判断解耦
- **向后兼容**：manifest 格式设计确保旧客户端能安全降级解析，新增字段使用 `Option` + 默认值
- **多源冗余**：GitHub 和 ModelScope 双源，自动延迟探测选最快响应

### 1.2 架构概览

```
用户执行 hermes update
        │
        ▼
┌─── 源选择（probe）────┐
│  并发探测延迟          │
│  GitHub vs ModelScope  │
└───────┬───────────────┘
        ▼
┌─── 获取 manifest ────┐
│  ReleaseManifest     │
│  (version/channel/   │
│   forced/platforms)  │
└───────┬──────────────┘
        ▼
┌─── 版本策略判断 ─────┐
│  SemverPolicy /      │
│  ChannelPolicy       │
│  → UpdateDecision    │
└───────┬──────────────┘
        ▼
  下载 → 校验 → 替换
```

### 1.3 为什么用 Strategy 模式

版本判断不是简单的"大于就更新"。不同场景需要不同策略：
- 生产环境只推 stable，不接受 pre-release
- 测试团队需要 beta/rc 渠道
- 安全漏洞需要跨版本强制更新
- 某些发行版可能锁定渠道

Strategy 模式让这些需求通过替换 Policy 实现，而非修改核心流程。

**架构优势：**
- **可插拔**：新增策略（如 `LockedChannelPolicy`、灰度策略）只需实现 `VersionPolicy` trait，无需改动调用方
- **可测试**：每个 Policy 独立单测，无需 mock 网络或文件系统
- **职责单一**：manifest 解析、渠道推导、版本比较、下载校验各司其职

### 1.4 已知架构限制（现状诚实说明）

> ⚠️ 以下是当前实现中存在的限制，维护者应在发版前了解：

| 限制 | 现状说明 |
|------|---------|
| Channel 硬编码 | `Channel` enum 固定为 `Stable/Rc/Beta/Nightly` 4 个变体，无动态扩展机制 |
| ChannelPolicy 已启用 | `ChannelPolicy` 已作为默认策略启用，客户端默认订阅 `Channel::Stable`，`--channel` 参数同时改变 manifest 来源和渠道级别过滤 |
| `--channel` 参数语义 | 不仅改变 manifest 获取来源，还通过 `ChannelPolicy` 做渠道级别过滤，纵深防御确保即使 manifest 内容错误也会被 Policy 层拦截 |
| GitHub 源已支持 forced/min_version | 通过 Release body 中的 `<!-- hermes-meta ... -->` HTML 注释块解析 `forced` 和 `min_version`（详见 §7） |
| SHA256 best-effort | 校验文件不可用时仅打印 warning 而不中止更新，不是强制校验 |
| Cosign 非强制 | 签名验证为可选步骤，客户端不强制验证 cosign 签名 |

---

## 二、CI 发布流水线详解

### 2.1 触发机制

CI 由 GitHub Actions 的 `push.tags` 事件触发，配置如下（`.github/workflows/release.yml`）：

```yaml
on:
  push:
    tags:
      - "v*"
```

**关键细节：**

- 使用 glob pattern `"v*"`，**任何以 `v` 开头的标签推送都会触发 CI**
- **已添加 `validate-tag` 前置校验**：CI 新增 `validate-tag` job 作为所有后续 job 的前置条件，使用正则 `^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9]+(\.[a-zA-Z0-9]+)*)?$` 校验 tag 格式
  - 无效格式（如 `vrelease`、`v1.2.a`、`v.1.0`）会 **fast-fail** 并输出清晰的错误信息
  - 所有后续 job（`security-release-gate`、`cross-build`、`macos-build`、`sign-and-publish`）均依赖 `validate-tag`

**正确的 Tag 格式：**

必须是 `v` + SemVer，允许 pre-release 后缀：

```bash
# ✅ 正确示例
git tag v1.2.0
git tag v1.2.0-beta.1
git tag v1.2.0-rc.1
git tag v1.2.0-nightly.20260604

# ❌ 错误示例（CI validate-tag 会直接拒绝）
git tag release-1.2.0   # 不以 v 开头，CI 不触发
git tag v1.2.a          # 无效 SemVer，validate-tag fast-fail
git tag vrelease        # 无版本号，validate-tag fast-fail
```

### 2.2 CI 完整流水线阶段

CI 共 5 个 job，按依赖顺序执行：

```
validate-tag
         │
         ▼
security-release-gate
         │
         ├──► cross-build (Linux/Windows)
         │
         └──► macos-build (macOS ARM + x86)
                   │
                   ▼
         sign-and-publish
```

#### 阶段 0：validate-tag

Tag 格式校验，是所有后续 job 的前置条件。使用 `grep -E` 正则校验 tag 是否符合 SemVer 格式：

```bash
# 校验正则（不含 v 前缀）
^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9]+(\.[a-zA-Z0-9]+)*)?$
```

- 无效格式会立即 fast-fail，输出清晰的错误信息
- 所有后续 job 均声明 `needs: validate-tag`，确保格式错误不会触发任何构建或发布

#### 阶段 1：security-release-gate

安全扫描与 SBOM 生成，是后续所有阶段的前置条件。

```yaml
steps:
  - Secret scan (tracked files)      # release_secret_scan.py 扫描仓库中的潜在密钥泄露
  - Config redaction tests (CLI)     # hermes-cli 敏感字段屏蔽测试
  - Config redaction tests (agent)   # hermes-agent 重放记录器哈希链测试
  - Generate SBOM (CycloneDX)        # 软件物料清单生成，用于供应链审计
```

产物（上传为 `release-security-artifacts`）：
- `.sync-reports/release-secret-scan.json`
- `release-sbom.cdx.json`

#### 阶段 2：cross-build（与 macos-build 并行）

使用 `cross` 工具链进行 Linux/Windows 交叉编译：

| 目标平台 | Rust Target | 产物文件名 |
|---------|-------------|-----------|
| Linux x86_64 (glibc) | `x86_64-unknown-linux-gnu` | `hermes-linux-x86_64.tar.gz` |
| Linux ARM64 (glibc) | `aarch64-unknown-linux-gnu` | `hermes-linux-aarch64.tar.gz` |
| Linux x86_64 (musl) | `x86_64-unknown-linux-musl` | `hermes-linux-x86_64-musl.tar.gz` |
| Windows x86_64 | `x86_64-pc-windows-gnu` | `hermes-windows-x86_64.zip` |

> **musl 说明**：`linux-x86_64-musl` 产物使用 musl libc 静态链接，可在 Alpine Linux、OpenWrt 等无 glibc 环境运行。

#### 阶段 3：macos-build（与 cross-build 并行）

在 GitHub-hosted macOS runner 上原生编译：

| 目标平台 | Rust Target | 产物文件名 |
|---------|-------------|-----------|
| macOS ARM64 (Apple Silicon) | `aarch64-apple-darwin` | `hermes-macos-aarch64.tar.gz` |
| macOS x86_64 (Intel) | `x86_64-apple-darwin` | `hermes-macos-x86_64.tar.gz` |

#### 阶段 4：sign-and-publish

汇总所有产物，签名、生成校验和，发布到 GitHub Release 并上传 ModelScope：

```
1. 下载所有构建产物 + 安全扫描产物
2. 生成 Homebrew formula（scripts/generate-homebrew-formula.sh）
3. Cosign keyless 签名所有 .tar.gz / .zip
4. 生成 checksums.sha256
5. 发布 GitHub Release（含所有 assets）
6. 上传至 ModelScope（artifacts + latest.json + channels/{channel}.json）
```

### 2.3 Cosign Keyless 签名机制

CI 使用 [Sigstore Cosign](https://docs.sigstore.dev/) 的 **keyless** 模式签名：

```bash
cosign sign-blob \
  --yes \
  --output-signature "${artifact}.sig" \
  --output-certificate "${artifact}.pem" \
  "${artifact}"
```

**工作原理：**
- 利用 GitHub Actions OIDC token（`id-token: write` 权限）向 Fulcio CA 申请短期证书
- 证书绑定 GitHub workflow identity（仓库 + ref + workflow name）
- 签名记录写入 Rekor 透明日志，任何人可验证
- **无需管理长期密钥**，消除了密钥泄露和轮换问题

**每个产物生成两个额外文件：**
- `{artifact}.sig` — 签名文件
- `{artifact}.pem` — 短期证书（含 OIDC 身份信息）

**客户端验证（可选，当前非强制）：**

```bash
cosign verify-blob \
  --signature hermes-linux-x86_64.tar.gz.sig \
  --certificate hermes-linux-x86_64.tar.gz.pem \
  --certificate-identity-regexp "hermes-agent-ultra" \
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
  hermes-linux-x86_64.tar.gz
```

### 2.4 SHA256 校验的 Best-Effort 行为

CI 生成的 `checksums.sha256` 随 release assets 一同发布。客户端校验逻辑（`crates/hermes-cli/src/update/verify.rs`）采用 best-effort 策略：

```rust
// 下载 checksums 文件失败时：打印 warn，跳过校验，继续更新
if !output.status.success() {
    tracing::warn!("Checksums file not available, skipping verification");
    return Ok(());  // 不中止，继续
}

// checksums 文件中找不到对应条目时：打印 warn，跳过
None => {
    tracing::warn!("No checksum entry for '{}' in checksums file, skipping verification", ...);
    return Ok(());  // 不中止，继续
}

// 哈希不匹配时：删除下载文件，报错中止 ✅（唯一的 hard fail 情况）
if actual_hash != expected_hash.to_lowercase() {
    let _ = std::fs::remove_file(archive_path);
    return Err(...);  // 中止
}
```

**设计动机：** 在网络不稳定或 ModelScope CDN 尚未同步 checksums 文件时，best-effort 策略避免更新完全阻塞；但若哈希不匹配（疑似篡改或下载损坏）则强制中止。

> ⚠️ **注意**：best-effort 意味着攻击者若能同时替换下载文件和 checksums 文件（如 MITM），校验无法防护。完整安全需结合 cosign 签名验证。

---

## 三、日常发版流程

### 3.1 发布 stable 版本

1. **确保 main 分支 CI 全绿**，然后打 tag：
   ```bash
   git checkout main
   git pull origin main
   git tag v1.2.0
   git push origin v1.2.0
   ```

2. **CI 自动完成**（见第二章流水线详解）：
   - 安全扫描 → 多平台编译 → cosign 签名 → GitHub Release 发布
   - ModelScope 上传：artifacts + manifest 写入 `hermes-agent-ultra/channels/stable.json` 和 `hermes-agent-ultra/latest.json`

3. **验证发布结果**：
   ```bash
   # 客户端验证
   hermes update --check
   # 应显示 "New version available: v1.2.0"

   # 检查 GitHub Release
   # https://github.com/Michael-Lfx/hermes-agent-ultra/releases/tag/v1.2.0

   # 检查 ModelScope manifest
   curl -s "https://modelscope.cn/api/v1/models/flowy2025/agent/repo?Revision=master&FilePath=hermes-agent-ultra/latest.json" | jq .
   ```

4. **Homebrew formula 验证**（若适用）：
   ```bash
   brew upgrade hermes-agent-ultra
   hermes --version
   ```

### 3.2 发布 beta/rc 版本

tag 名称带 pre-release 后缀，系统自动识别渠道（详见第四章渠道推导规则）：

```bash
# Beta
git tag v1.3.0-beta.1
git push origin v1.3.0-beta.1

# Release Candidate
git tag v1.3.0-rc.1
git push origin v1.3.0-rc.1

# Nightly（通常由 CI 自动打 tag）
git tag v1.3.0-nightly.20260604
git push origin v1.3.0-nightly.20260604
```

CI 会将 manifest 写入对应渠道路径（如 `hermes-agent-ultra/channels/beta.json`）。

### 3.3 用户侧切换渠道

```bash
# 检查 beta 渠道
hermes update --check --channel beta

# 更新到 beta 最新
hermes update --channel beta

# 回到 stable
hermes update --channel stable
```

> **注意**：客户端已默认使用 `ChannelPolicy`（订阅 `Channel::Stable`），`--channel` 参数同时改变 manifest 获取路径和渠道级别过滤策略。纵深防御确保即使 manifest 返回了错误渠道的版本，Policy 层也会拦截。

### 3.4 强制指定更新源

```bash
hermes update --source github
hermes update --source modelscope
```

或通过环境变量持久化：
```bash
export HERMES_UPDATE_SOURCE=modelscope
```

### 3.5 错误 Tag 的紧急修复

如果误推了格式错误的 tag（如 `vrelease`、`v1.2.a`），需立即处理：

**第一步：删除错误 tag（本地 + 远端）**

```bash
# 删除远端 tag
git push origin :refs/tags/vrelease

# 删除本地 tag
git tag -d vrelease
```

**第二步：清理 GitHub Release**

如果 CI 已生成 Release，需手动到 GitHub 删除：
1. 进入 `https://github.com/Michael-Lfx/hermes-agent-ultra/releases`
2. 找到对应 release，点击 Delete

**第三步：清理 ModelScope**

如果 `latest.json` 已被错误版本覆盖，需要手动上传正确的 manifest：

```bash
# 找到最近一次正确版本的 manifest
# 用 upload-modelscope-release.py 重新上传
python3 scripts/upload-modelscope-release.py \
  --repo flowy2025/agent \
  --version v1.2.0 \
  --dist-dir dist/
```

**第四步：打正确 tag 重新发布**

```bash
git tag v1.2.1  # 或其他正确版本号
git push origin v1.2.1
```

> **预防措施**：CI 已添加 `validate-tag` 前置校验 job，格式错误的 tag 会在构建前被拒绝。建议同时在本地添加 git hook 校验 tag 格式（`.scripts/git-hooks/` 目录）作为额外防线。

---

## 三-A、Nightly 自动构建

### 触发机制

- **定时触发**：每日 UTC 16:00（北京时间 00:00）自动执行
- **手动触发**：支持 GitHub Actions `workflow_dispatch` 手动触发
- **Workflow 文件**：`.github/workflows/nightly.yml`

### 与正式 Release 的隔离

- `release.yml` 通过 `!v*-nightly.*` 排除规则，nightly tag 不会触发正式发布流程
- Nightly 不会创建 GitHub Release（避免噪音）
- Nightly 直接上传到 ModelScope nightly 渠道

### Tag 命名与保留策略

- **命名格式**：`v0.1.0-nightly.YYYYMMDD`（如 `v0.1.0-nightly.20260605`）
- **保留策略**：仅保留最近 7 个 nightly tag，超出部分自动清理
- **同日重跑**：同一天重复触发会先删除同名 tag 再重建

### Nightly 构建流水线

```
prepare (创建 tag + 清理旧 tag)
    ├── cross-build (Linux x86_64, aarch64, musl + Windows)
    └── macos-build (aarch64, x86_64)
            └── modelscope-upload (打包 + 上传 + 清理旧 artifact)
```

| 阶段 | 操作 | 说明 |
|------|------|------|
| prepare | 创建 nightly tag | 格式 `v0.1.0-nightly.YYYYMMDD`，清理 >7 个的旧 tag |
| cross-build | 多平台编译 | Linux (x86_64, aarch64, musl) + Windows |
| macos-build | macOS 编译 | aarch64 + x86_64 |
| modelscope-upload | 上传到 ModelScope | 生成 checksums + 上传 manifest 到 nightly channel |

### ModelScope Artifact 清理

- 脚本：`scripts/cleanup-modelscope-nightly.py`
- 保留最近 7 个 nightly 版本的 artifact
- 支持 `--dry-run` 预览模式
- 支持 `--keep N` 自定义保留数量

### 用户获取 Nightly 版本

```bash
# 检查 nightly 渠道最新版本
hermes update --check --channel nightly

# 更新到 nightly 最新
hermes update --channel nightly

# 回到 stable
hermes update --channel stable
```

### Nightly 分支配置说明

> **ℹ️ 已合并分支清理记录**：在 `feat/wecom_websocket` 合并到 `main` 之前，nightly 构建的 prepare job checkout 步骤中曾硬编码 `ref: feat/wecom_websocket`，以确保 nightly 能构建该 feature 分支的代码。合并后该 `ref` 配置已移除（连同 TODO 注释），nightly 现在默认使用仓库的默认分支（`main`）进行 checkout。cross-build 和 macos-build job 始终使用动态 ref（`${{ needs.prepare.outputs.nightly-tag }}`），无需额外修改。

### Nightly 故障排查

| 症状 | 可能原因 | 解决方法 |
|------|---------|----------|
| Nightly CI 未触发 | GitHub schedule 延迟（正常30分钟内） | 手动 workflow_dispatch 触发 |
| Tag 创建失败 | 权限不足 | 检查 `GITHUB_TOKEN` permissions 是否包含 `contents: write` |
| ModelScope 上传失败 | `MODELSCOPE_TOKEN` 未配置 | 在 repo Settings > Secrets 中配置 |
| 旧 tag 未清理 | GitHub API rate limit | 下次运行会补偿清理 |
| Nightly 编译的不是最新代码 | `ref: feat/wecom_websocket` 硬编码未移除（已修复） | 已从 nightly.yml 的 prepare checkout 中删除 `ref` 行，现默认使用 main 分支 |

---

## 四、渠道（Channel）推导规则详解

### 4.1 推导算法

渠道从 tag 的 pre-release 字符串推导，使用**字符串包含（contains）**判断，大小写不敏感。

**Rust 端实现**（`crates/hermes-cli/src/update/version.rs`）：

```rust
impl Channel {
    pub fn from_prerelease(pre: &str) -> Self {
        let lower = pre.to_lowercase();
        if lower.contains("nightly") {
            Channel::Nightly
        } else if lower.contains("beta") {
            Channel::Beta
        } else if lower.contains("rc") {
            Channel::Rc
        } else if pre.is_empty() {
            Channel::Stable
        } else {
            Channel::Beta // 未知 pre-release 默认视为 beta
        }
    }
}
```

**Python 端实现**（`scripts/upload-modelscope-release.py`）：

```python
def derive_channel(version: str) -> str:
    v_lower = version.lower()
    if "nightly" in v_lower:
        return "nightly"
    if "beta" in v_lower:
        return "beta"
    if "rc" in v_lower:
        return "rc"
    # 只有纯版本号（无 pre-release）才返回 stable
    # 未知后缀统一返回 beta（与 Rust 侧一致）
    parsed = parse_version(version)
    if not parsed.pre:
        return "stable"
    print(f"WARNING: Unknown pre-release suffix in '{version}', defaulting to 'beta'")
    return "beta"
```

> ✅ **Rust 与 Python 端已统一**：两侧对未知后缀均默认返回 `beta`。Python 端仅在无 pre-release 时返回 `stable`，有未知后缀时返回 `beta` 并打印 WARNING。Rust 侧 `from_prerelease()` 在命中默认 Beta 分支时也会通过 `tracing::warn!` 输出警告。

### 4.2 优先级与判断顺序

判断按**短路求值**顺序执行，先命中者胜出：

```
1. nightly  → Channel::Nightly（最高优先级）
2. beta     → Channel::Beta
3. rc       → Channel::Rc
4. (空字符串) → Channel::Stable
5. (其它任何) → Channel::Beta（兜底默认）
```

### 4.3 各种 Tag 的渠道推导示例

| Tag | Pre-release 部分 | 包含关键词 | 推导结果 | 说明 |
|-----|-----------------|-----------|---------|------|
| `v1.0.0` | `""` (空) | — | **Stable** | 无 pre-release，正式稳定版 |
| `v1.0.0-beta.1` | `"beta.1"` | `beta` | **Beta** | 标准 beta |
| `v1.0.0-rc.1` | `"rc.1"` | `rc` | **Rc** | 标准 RC |
| `v1.0.0-nightly.42` | `"nightly.42"` | `nightly` | **Nightly** | 标准 nightly |
| `v1.0.0-test` | `"test"` | — | **Beta** ⚠️ | 未知后缀，默认 Beta |
| `v1.0.0-alpha.1` | `"alpha.1"` | — | **Beta** ⚠️ | alpha 未被识别，默认 Beta |
| `v1.0.0-oem.1` | `"oem.1"` | — | **Beta** ⚠️ | OEM 未被识别，默认 Beta |
| `v1.0.0-internal` | `"internal"` | — | **Beta** ⚠️ | 内部版本被归为 Beta |
| `v1.0.0-rc-beta` | `"rc-beta"` | `beta` > `rc` | **Beta** | beta 优先级高于 rc |
| `v1.0.0-NIGHTLY` | `"NIGHTLY"` | `nightly`（大小写不敏感） | **Nightly** | 大写也被识别 |

### 4.4 未知后缀的风险分析

当前架构将所有非标准后缀压缩到 Beta 渠道，这带来以下风险：

- **OEM 版本无法区分**：`v1.0.0-oem.1` 和 `v1.0.0-beta.1` 同属 Beta 渠道，OEM 用户可能收到真正的 beta 版本
- **alpha 测试混淆**：alpha 测试版本被归入 Beta，无法通过渠道隔离
- **内部版本泄露**：`v1.0.0-internal` 走 Beta 渠道，可能推送给外部 beta 用户
- **兜底行为不可见**：代码注释明确写了"未知 pre-release 默认视为 beta"，但维护者易忽视

> **建议**：如果有多种 pre-release 类型共存的需求，应扩展 `Channel` enum（见 §4.5）。

### 4.5 如何扩展渠道（以 OEM 为例）

若需要新增 OEM 渠道，需修改以下位置：

#### 步骤 1：扩展 Channel enum

文件：`crates/hermes-cli/src/update/version.rs`

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Default)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    #[default]
    Stable,
    Rc,
    Beta,
    Nightly,
    Oem,    // ← 新增
}
```

#### 步骤 2：更新 `from_prerelease()`

```rust
pub fn from_prerelease(pre: &str) -> Self {
    let lower = pre.to_lowercase();
    if lower.contains("nightly") {
        Channel::Nightly
    } else if lower.contains("oem") {       // ← 新增（注意顺序）
        Channel::Oem
    } else if lower.contains("beta") {
        Channel::Beta
    } else if lower.contains("rc") {
        Channel::Rc
    } else if pre.is_empty() {
        Channel::Stable
    } else {
        Channel::Beta
    }
}
```

> **顺序很重要**：`oem` 必须在 `beta` 之前检测，否则 `v1.0.0-oem-beta` 会被误归为 Beta。

#### 步骤 3：更新 `from_str()`

```rust
pub fn from_str(s: &str) -> Self {
    match s.to_lowercase().as_str() {
        "stable" => Channel::Stable,
        "rc" => Channel::Rc,
        "beta" => Channel::Beta,
        "nightly" => Channel::Nightly,
        "oem" => Channel::Oem,    // ← 新增
        _ => Channel::Stable,
    }
}
```

#### 步骤 4：更新 Python 上传脚本

文件：`scripts/upload-modelscope-release.py`

```python
def derive_channel(version: str) -> str:
    v_lower = version.lower()
    if "oem" in v_lower:        # ← 新增（注意顺序）
        return "oem"
    if "beta" in v_lower:
        return "beta"
    if "rc" in v_lower:
        return "rc"
    if "nightly" in v_lower:
        return "nightly"
    return "stable"
```

#### 步骤 5：更新 ChannelPolicy 过滤矩阵

```rust
Channel::Oem => {
    // OEM 用户只接收 stable + oem，不接收 beta/rc/nightly
    if available_channel != Channel::Stable && available_channel != Channel::Oem {
        return UpdateDecision::DoNotUpdate { ... };
    }
}
```

#### 步骤 6：更新测试与文档

- 在 `version.rs` 的 `#[cfg(test)]` 中添加 OEM 相关用例
- 更新本文档的渠道推导示例表
- 更新 ModelScope 仓库结构说明（新增 `channels/oem.json`）

---

## 五、Manifest 格式规范

### 5.1 完整格式

```json
{
  "version": "1.2.0",
  "channel": "stable",
  "pub_date": "2026-06-03T12:00:00Z",
  "forced": false,
  "min_version": "0.10.0",
  "notes": "What's new in this release...",
  "platforms": {
    "linux-x86_64": {
      "url": "https://..../hermes-linux-x86_64.tar.gz",
      "sha256": "abcd1234...",
      "size": 12345678
    },
    "windows-x86_64": {
      "url": "https://..../hermes-windows-x86_64.zip",
      "sha256": "efgh5678...",
      "size": 9876543
    },
    "macos-aarch64": { "url": "...", "sha256": "...", "size": 11000000 },
    "macos-x86_64": { "url": "...", "sha256": "...", "size": 10500000 },
    "linux-aarch64": { "url": "...", "sha256": "...", "size": 11200000 },
    "linux-x86_64-musl": { "url": "...", "sha256": "...", "size": 8500000 }
  },
  "artifacts": [
    "hermes-linux-x86_64.tar.gz",
    "hermes-linux-aarch64.tar.gz",
    "hermes-linux-x86_64-musl.tar.gz",
    "hermes-windows-x86_64.zip",
    "hermes-macos-aarch64.tar.gz",
    "hermes-macos-x86_64.tar.gz"
  ]
}
```

### 5.2 字段说明

| 字段 | 必填 | 类型 | 说明 |
|------|------|------|------|
| `version` | 是 | string | SemVer 版本号（不带 `v` 前缀），如 `"1.2.0"` 或 `"1.0.0-beta.1"` |
| `channel` | 否 | string | 默认 `"stable"`。从 version pre-release 自动推导 |
| `pub_date` | 否 | string | ISO 8601 / RFC 3339 发布时间 |
| `forced` | 否 | bool | 设为 `true` 时客户端强制更新（用于安全补丁） |
| `min_version` | 否 | string | 低于此版本的客户端会被强制更新 |
| `notes` | 否 | string | Release notes |
| `platforms` | 是(新格式) | object | 按平台的下载信息，key 为平台标识，value 含 `url`/`sha256`/`size` |
| `artifacts` | 否 | string[] | 文件名列表（向后兼容旧客户端） |
| `tag` | 否 | string | 旧格式的 tag 字段（兼容），如 `"v1.0.0"` |

### 5.3 平台 key 映射

| 平台 | Key | 编译 Target | 打包格式 |
|------|-----|------------|---------|
| Linux x86_64 (glibc) | `linux-x86_64` | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| Linux ARM64 (glibc) | `linux-aarch64` | `aarch64-unknown-linux-gnu` | `.tar.gz` |
| Linux x86_64 (musl) | `linux-x86_64-musl` | `x86_64-unknown-linux-musl` | `.tar.gz` |
| Windows x86_64 | `windows-x86_64` | `x86_64-pc-windows-gnu` | `.zip` |
| macOS ARM64 (Apple Silicon) | `macos-aarch64` | `aarch64-apple-darwin` | `.tar.gz` |
| macOS x86_64 (Intel) | `macos-x86_64` | `x86_64-apple-darwin` | `.tar.gz` |

### 5.4 向后兼容

旧客户端（未升级版本管理模块的版本）：
- 只读取 `version` 和 `artifacts` 字段
- 忽略不认识的字段（`platforms`/`forced`/`min_version` 等）
- 仍然可以正常检测更新和下载

**Rust 端反序列化实现**（`crates/hermes-cli/src/update/manifest.rs`）：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseManifest {
    pub version: String,
    #[serde(default = "default_stable")]
    pub channel: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pub_date: Option<String>,
    #[serde(default)]
    pub forced: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_version: Option<String>,
    // ... platforms, artifacts, tag
}
```

所有可选字段使用 `Option<T>` + `#[serde(default)]`，确保旧格式 JSON 能安全反序列化。

### 5.5 Schema 版本建议

当前 manifest 没有显式的 `schema_version` 字段。若未来需要破坏性变更格式，建议新增：

```json
{
  "schema_version": 2,
  "version": "2.0.0",
  ...
}
```

客户端通过 `schema_version` 判断是否能解析该 manifest：
- 缺失或 `1`：当前格式
- `2`：需要新版客户端
- 客户端遇到未知的 `schema_version` 应提示升级而非报错

---

## 六、版本策略配置

### 6.1 更新决策逻辑

```
available > current → 建议更新
available == current → 已是最新
available < current → 不更新（除非 forced=true）
current < min_version → 强制更新
available 在 deprecated 列表中 → 不更新
```

**实现参考**（`SemverPolicy`，`crates/hermes-cli/src/update/version.rs`）：

```rust
impl VersionPolicy for SemverPolicy {
    fn evaluate(&self, current: &Version, available: &Version, meta: &UpdateMeta) -> UpdateDecision {
        // 1. 检查 deprecated
        if meta.deprecated_versions.contains(available) {
            return UpdateDecision::DoNotUpdate { ... };
        }
        // 2. 检查 min_supported_version
        if let Some(ref min) = meta.min_supported_version {
            if current < min {
                return UpdateDecision::UpdateAvailable { forced: true };
            }
        }
        // 3. forced 优先
        if meta.forced && available != current {
            return UpdateDecision::UpdateAvailable { forced: true };
        }
        // 4. 标准 SemVer 比较
        match available.cmp(current) { ... }
    }
}
```

### 6.2 强制更新场景

当发现安全漏洞需要所有用户紧急升级时：

1. 在 manifest 中设置 `"forced": true`
2. 或设置 `"min_version": "1.1.5"`（低于此版本的客户端自动强制更新）

客户端行为：
- 显示 `[FORCED UPDATE REQUIRED]` 提示
- 仍然会询问确认（除非带 `-y`），但跳过版本比较

**操作方式**：发布新版本时，手动修改 ModelScope 上已有 manifest 的 `forced` 字段，或设置 `min_version`。当前 CI 不自动设置这些字段，需人工干预。

### 6.3 渠道过滤规则（设计意图 vs 实际行为）

**设计意图**（`ChannelPolicy` 实现）：

| 用户渠道 | 能收到的更新 |
|----------|-------------|
| stable | 仅 stable 正式版 |
| rc | stable + rc |
| beta | stable + rc + beta |
| nightly | 所有版本 |

**实际行为**：

> ✅ 客户端已默认使用 `ChannelPolicy`（默认订阅 `Channel::Stable`）。渠道过滤在 Policy 层生效，不再仅依赖 manifest 文件分离。`--channel` 参数同时改变 manifest 获取路径和版本比较层面的渠道过滤。

纵深防御机制：即使 manifest 内容返回了错误渠道的版本（如 stable manifest 中混入了 beta 版本），`ChannelPolicy` 也会在 Policy 层拦截不匹配渠道的版本，返回 `DoNotUpdate`。

---

## 七、GitHub vs ModelScope 元数据差异

两个更新源在功能支持上存在差异：

| 特性 | GitHub Release | ModelScope |
|------|---------------|------------|
| Manifest 获取 | 通过 GitHub API 读取 release assets | 直接下载 JSON 文件 |
| `forced` 字段 | ✅ 支持（通过 Release body 的 `hermes-meta` 注释块） | ✅ 支持 |
| `min_version` 字段 | ✅ 支持（通过 Release body 的 `hermes-meta` 注释块） | ✅ 支持 |
| `platforms` 字段 | ✅ 支持 | ✅ 支持 |
| 渠道分离（channels/） | ❌ 仅 `latest.json` | ✅ `channels/{channel}.json` |
| SHA256 校验 | ✅ 从 `checksums.sha256` asset 获取 | ✅ 从 manifest `platforms[].sha256` 获取 |
| Cosign 签名文件 | ✅ `.sig` + `.pem` 随 release assets | ❌ 不上传签名文件 |
| SBOM | ✅ `release-sbom.cdx.json` | ❌ 不上传 |
| Homebrew formula | ✅ 附带 | ❌ 不附带 |
| CDN 缓存 | 无缓存 | 可能有延迟 |
| 私有仓库认证 | `GITHUB_TOKEN` | `MODELSCOPE_TOKEN` |

**GitHub 源 forced/min_version 支持**：

GitHub 源通过解析 Release body 中的 `<!-- hermes-meta ... -->` HTML 注释块获取元数据。发布时在 Release body 中添加如下注释即可：

```html
<!-- hermes-meta
forced: true
min_version: 1.1.5
-->
```

- `forced`：设为 `true` 时客户端强制更新（用于安全补丁），默认 `false`
- `min_version`：低于此版本的客户端会被强制更新，默认 `None`
- 无 meta block 时保持默认值（向后兼容）
- 解析函数：`parse_release_meta()`（`crates/hermes-cli/src/update/github.rs`）

**建议**：两个源功能已对齐，可根据网络环境自由选择。ModelScope 在国内网络更稳定，GitHub 在国际网络更快捷。

---

## 八、ModelScope 仓库结构

```
flowy2025/agent (model repo)
└── hermes-agent-ultra/
    ├── latest.json                          ← stable 最新版 manifest（别名）
    ├── channels/
    │   ├── stable.json                      ← stable 渠道 manifest
    │   ├── beta.json                        ← beta 渠道 manifest
    │   ├── rc.json                          ← rc 渠道 manifest（如有）
    │   └── nightly.json                     ← nightly 渠道 manifest
    ├── v1.2.0/
    │   ├── hermes-linux-x86_64.tar.gz
    │   ├── hermes-linux-aarch64.tar.gz
    │   ├── hermes-linux-x86_64-musl.tar.gz
    │   ├── hermes-windows-x86_64.zip
    │   ├── hermes-macos-aarch64.tar.gz
    │   ├── hermes-macos-x86_64.tar.gz
    │   ├── checksums.sha256
    │   ├── install.sh
    │   └── hermes-agent-ultra.rb
    └── v1.3.0-beta.1/
        └── ...
```

> **多项目前缀设计**：`hermes-agent-ultra/` 前缀使同一 ModelScope 仓库可服务多个项目（如未来的 hermes-tui、hermes-gateway），只需更换前缀。

---

## 九、环境变量参考

| 变量 | 作用 | 默认值 |
|------|------|--------|
| `HERMES_UPDATE_SOURCE` | 强制指定源 (`github`/`modelscope`) | 自动探测（延迟选最快） |
| `HERMES_UPDATE_REPO` | GitHub 仓库地址 | `Michael-Lfx/hermes-agent-ultra` |
| `HERMES_MODELSCOPE_REPO` | ModelScope 仓库地址 | `flowy2025/agent` |
| `GITHUB_TOKEN` | GitHub API 认证（私有仓库必需） | 无 |
| `MODELSCOPE_TOKEN` | ModelScope 上传认证（CI 使用） | 无 |
| `RUST_LOG` | 日志级别（调试时用） | 无 |

---

## 十、扩展方向

### 10.1 发行版渠道锁定

某些场景下希望分发的 binary 只能使用特定渠道（如 OEM 版本锁定 stable）：

- 通过 Cargo feature flag 在编译时注入锁定渠道
- 运行时优先级：编译锁定 > 服务端下发 > 用户参数
- 当前 Strategy 模式天然支持，新增一个 `LockedChannelPolicy` 包装即可

### 10.2 渐进式发布 (Progressive Rollout)

逐步扩大推送范围，降低风险：

- manifest 新增 `rollout_percentage` 字段（如 10% → 50% → 100%）
- 客户端根据设备 ID hash 决定是否命中灰度
- 可通过 `metadata` 扩展字段承载，不破坏现有格式

**实现思路：**

```json
{
  "version": "1.3.0",
  "rollout": {
    "percentage": 10,
    "hash_seed": "20260604"
  }
}
```

客户端计算 `sha256(device_id + hash_seed) % 100 < percentage` 决定是否提示更新。

### 10.3 增量更新 (Delta Update)

当前为全量替换 binary。未来可优化：

- 使用 bsdiff/bspatch 生成差分包
- manifest 的 `platforms` 中新增 `delta_url` + `delta_from_version` 字段
- 客户端判断是否有匹配的 delta，fallback 为全量

### 10.4 自动回滚

更新后如果新版本启动崩溃：

- 当前已支持 `hermes update --rollback` 手动回滚
- 可扩展为：新版本首次启动写入 "pending" 标记，稳定运行 N 秒后确认；崩溃时自动恢复 `.bak`

### 10.5 版本回溯审计

记录每次更新的历史：

- 本地存储 `~/.hermes/update-history.json`
- 包含时间戳、from/to 版本、来源、成功/失败
- 用于诊断和 telemetry

### 10.6 多产品共享基础设施

当前 ModelScope 仓库已设计为多项目前缀（`hermes-agent-ultra/`）。未来其他产品可复用：

- 相同的 manifest 格式
- 相同的 CI 上传脚本（改 `--prefix` 参数即可）
- 客户端共享 `version.rs` 和 `manifest.rs` 模块

### 10.7 渠道动态扩展机制

当前 `Channel` 硬编码为 4 个变体。未来可重构为：

```rust
// 方案 A：String-based channel（灵活但丢失类型安全）
pub struct Channel(String);

// 方案 B：Known + Custom（保留常见类型，允许扩展）
pub enum Channel {
    Stable, Rc, Beta, Nightly,
    Custom(String),
}
```

---

## 十一、故障排查

### 常见问题速查表

| 症状 | 可能原因 | 解决方法 |
|------|---------|---------|
| "No artifact for platform" | manifest 缺少当前平台 | 检查 CI 是否成功构建该平台；检查 `platforms` 字段是否包含对应 key |
| 版本检查超时 | 网络问题或源不可达 | 用 `--source` 指定可达的源；检查代理设置 |
| "Already up to date" 但实际有新版 | 旧 manifest 缓存 | ModelScope CDN 可能有缓存延迟，等几分钟后重试 |
| 强制更新不生效 | manifest `forced=false` | 检查上传脚本生成的 manifest；手动修改 ModelScope 上的文件 |
| channel 参数被忽略 | 检查 `--channel` 参数值是否正确 | `--channel` 现在通过 `ChannelPolicy` 做渠道过滤，确认参数值与期望渠道一致 |
| CI 未触发 | tag 不以 `v` 开头 | 确认 tag 格式为 `v*`，如 `v1.2.0` |
| CI 跑了但版本号显示 0.0.0 | tag 不是有效 SemVer | 删除错误 tag，打正确格式（见 §3.5）；CI `validate-tag` 已拦截大部分格式错误 |
| CI tag 校验失败 | tag 格式不符合 SemVer | 确认 tag 格式为 `v` + `MAJOR.MINOR.PATCH[-prerelease]`；`validate-tag` 使用正则校验，仅允许数字版本号和字母数字 pre-release 后缀 |
| GitHub Release 元数据不生效 | Release body 缺少 `hermes-meta` 注释块 | 检查 Release body 是否包含 `<!-- hermes-meta ... -->` 块；格式必须为 HTML 注释，`forced` 和 `min_version` 各占一行 |
| ModelScope 上传失败 | `MODELSCOPE_TOKEN` 未配置或过期 | 检查 GitHub Actions secrets 中的 `MODELSCOPE_TOKEN` |
| SHA256 校验跳过 | checksums 文件不可用 | 查看 debug 日志确认；best-effort 设计下不影响更新 |
| Cosign 验证失败 | OIDC 证书过期 | keyless 证书短期有效，正常现象；不影响客户端更新 |
| 推送 tag 后 Release 页面为空 | security-release-gate 失败 | 检查 Actions 日志，通常是 secret scan 或测试失败 |

### 调试方法

```bash
# 查看详细日志（调试更新流程）
RUST_LOG=debug hermes update --check

# 强制指定源排查
hermes update --check --source modelscope
hermes update --check --source github

# 查看当前版本信息
hermes --version

# 手动获取 manifest 检查内容
curl -s "https://modelscope.cn/api/v1/models/flowy2025/agent/repo?Revision=master&FilePath=hermes-agent-ultra/latest.json" | python3 -m json.tool

# 检查 GitHub Release assets
gh release view v1.2.0 --json assets --jq '.assets[].name'

# 验证 cosign 签名（可选）
cosign verify-blob \
  --signature hermes-linux-x86_64.tar.gz.sig \
  --certificate hermes-linux-x86_64.tar.gz.pem \
  --certificate-identity-regexp "hermes-agent-ultra" \
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
  hermes-linux-x86_64.tar.gz
```

### CI 失败排查清单

1. **security-release-gate 失败**
   - secret scan 发现疑似密钥泄露 → 检查 `release_secret_scan.py` 报告
   - redaction 测试失败 → 确保敏感字段在日志中被正确屏蔽
   - SBOM 生成失败 → 检查 `cargo-cyclonedx` 版本兼容性

2. **cross-build / macos-build 失败**
   - 编译错误 → 检查 Actions 日志中的 cargo 错误信息
   - cross 工具链问题 → 确认 `cross` 镜像可用，检查 `Cross.toml` 配置
   - 超时 → 大版本首次编译可能超时，考虑增加 Actions timeout

3. **sign-and-publish 失败**
   - cosign 签名失败 → 检查 `id-token: write` 权限是否正确
   - GitHub Release 创建失败 → 检查 `contents: write` 权限
   - ModelScope 上传失败 → 确认 `MODELSCOPE_TOKEN` secret 有效
