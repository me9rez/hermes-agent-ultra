# Weixin (WeChat) Gateway 配置指南

## 1. 概述

### 什么是 Weixin Gateway

Weixin Gateway 是 Hermes Agent 的微信消息接入层，通过 Tencent **iLink Bot API** 将个人微信账号连接到 Hermes 智能体。它使微信用户可以直接与 Hermes Agent 对话，Agent 的回复也会通过微信送达。

### 基于 Tencent iLink Bot API

iLink Bot 是腾讯提供的个人微信机器人接入方案。QR 码登录后，Hermes 连接的是一个 **iLink bot 身份**（例如 `a5ace6fd482e@im.bot`），而非直接操作用于扫码的个人微信账号。这意味着：

- iLink bot 身份与扫码用的微信账号是**两个独立身份**。
- iLink bot 通常**无法被邀请进入普通微信群聊**。
- iLink 通常**不会将普通微信群聊消息**（包括对扫码账号的 @提及）转发给 bot。
- 实际部署中，大多数场景只有 DM（私聊）能可靠工作。

> 如需企业微信接入，请使用 WeCom 适配器，而非本适配器。

### 通信机制

Weixin Gateway 使用 **HTTP 长轮询**（long-polling）机制接收消息：

1. **连接**：验证凭证后启动轮询循环。
2. **轮询**：调用 `getupdates` 接口，超时设为 35 秒；服务端在有消息到达或超时前保持请求挂起。
3. **分发**：收到的消息通过 `tokio::spawn` 并发分发处理。
4. **同步游标**：`get_updates_buf` 持久化到磁盘，重启后从正确位置恢复。

无需公网端口、Webhook 或 WebSocket。

---

## 2. 交互式配置流程 (`hermes gateway setup`)

### 启动向导

```bash
hermes gateway setup
```

在平台列表中选择 **Weixin / WeChat**。

### 步骤一：QR 码登录

向导自动调用 `hermes auth login weixin --qr`，执行 QR 登录流程：

1. 向 iLink Bot API 的 `ilink/bot/get_bot_qrcode` 端点请求 QR 码（参数 `bot_type=3`）。
2. 在终端渲染 QR 码（同时打印 URL 供无法渲染终端使用）。
3. 提示：`请使用微信扫描二维码，并在手机端确认登录。`
4. 以 1 秒间隔轮询 `ilink/bot/get_qrcode_status` 端点，总超时 480 秒。

QR 码状态及其含义：

| 状态 | 行为 |
|------|------|
| `wait` | 等待扫码，继续轮询 |
| `scaned` | 已扫码，终端显示 `已扫码，请在微信里确认...` |
| `scaned_but_redirect` | 已扫码但需要重定向，自动切换到 `redirect_host` |
| `expired` | QR 码过期，自动刷新（最多 3 次），显示 `二维码已过期，正在刷新... (N/3)` |
| `confirmed` | 登录确认成功，提取凭证 |

超过 3 次过期将报错 `weixin qr expired too many times` 并退出。

登录成功后提取以下信息：

- `ilink_bot_id` / `account_id` -- iLink Bot 账号 ID
- `bot_token` / `token` -- iLink Bot 令牌
- `baseurl` -- API 基础 URL（可能与请求时不同，服务端可能重定向）
- `ilink_user_id` / `user_id` -- iLink 用户 ID

### 步骤二：DM 策略选择

登录成功后，向导提示 DM（私聊）访问策略：

```
Direct message policy: 1)pairing 2)open 3)allowlist 4)disabled
Choose [1-4] (default 1):
```

| 选项 | 写入字段 | 值 |
|------|----------|-----|
| 1（默认） | `platforms.weixin.extra.dm_policy` | `"pairing"` |
| 2 | `platforms.weixin.extra.dm_policy` | `"open"` |
| 3 | `platforms.weixin.extra.dm_policy` | `"allowlist"` |
| 4 | `platforms.weixin.extra.dm_policy` | `"disabled"` |

选择 3（allowlist）时，会额外提示输入允许的用户 ID：

```
Allowed Weixin user IDs (comma-separated):
```

写入 `platforms.weixin.extra.allow_from`，为 JSON 字符串数组。

### 步骤三：群组策略选择

```
Group policy: 1)disabled 2)open 3)allowlist
Choose [1-3] (default 1):
```

| 选项 | 写入字段 | 值 |
|------|----------|-----|
| 1（默认） | `platforms.weixin.extra.group_policy` | `"disabled"` |
| 2 | `platforms.weixin.extra.group_policy` | `"open"` |
| 3 | `platforms.weixin.extra.group_policy` | `"allowlist"` |

选择 3（allowlist）时，会额外提示输入允许的群组 ID：

```
Allowed Weixin group IDs (comma-separated):
```

写入 `platforms.weixin.extra.group_allow_from`，为 JSON 字符串数组。

### 步骤四：Home Channel（可选）

```
Weixin home channel (optional):
```

输入一个 chat ID，用于定时任务和通知的输出通道。写入 `platforms.weixin.home_channel`。

### 登录成功后保存的数据

QR 登录成功后，凭证同时保存到两个位置：

**1. config.yaml**（Hermes 配置文件）：

```yaml
platforms:
  weixin:
    enabled: true
    token: "your-bot-token"
    extra:
      account_id: "your-account-id"
      base_url: "https://ilinkai.weixin.qq.com"
      dm_policy: "pairing"
      group_policy: "disabled"
      allow_from: []
      group_allow_from: []
```

**2. 账号凭证文件**（`~/.hermes/weixin/accounts/<account_id>.json`）：

```json
{
  "token": "your-bot-token",
  "base_url": "https://ilinkai.weixin.qq.com",
  "user_id": "your-user-id",
  "saved_at": "2026-06-01T12:00:00Z"
}
```

终端输出确认信息：

```
Weixin: account_id/token saved and platform enabled in /path/to/config.yaml
```

---

## 3. 配置文件详解

### config.yaml 中 `platforms.weixin` 完整字段

```yaml
platforms:
  weixin:
    enabled: true                          # 是否启用 Weixin 适配器
    token: "bot-token-here"                # iLink Bot token（必需）
    home_channel: "chat-id-here"           # Home channel chat ID（可选）
    extra:
      account_id: "account-id-here"        # iLink Bot 账号 ID（必需）
      token: "fallback-token"              # token 回退值（优先使用顶层 token）
      base_url: "https://ilinkai.weixin.qq.com"   # iLink API 基础 URL
      cdn_base_url: "https://novac2c.cdn.weixin.qq.com/c2c"  # CDN 基础 URL
      dm_policy: "open"                    # DM 访问策略
      group_policy: "disabled"             # 群组访问策略
      allow_from:                          # DM 白名单用户 ID 列表
        - "user_id_1"
        - "user_id_2"
      group_allow_from:                    # 群组白名单 ID 列表
        - "group_id_1"
```

### WeixinConfig 结构体定义

Rust 结构体 `WeixinConfig` 定义如下：

```rust
pub struct WeixinConfig {
    pub account_id: String,       // 必填
    pub token: String,            // 必填
    pub base_url: String,         // 默认 "https://ilinkai.weixin.qq.com"
    pub cdn_base_url: String,     // 默认 "https://novac2c.cdn.weixin.qq.com/c2c"
    pub dm_policy: String,        // 默认 "open"
    pub group_policy: String,     // 默认 "disabled"
    pub allow_from: Vec<String>,  // 默认空
    pub group_allow_from: Vec<String>,  // 默认空
    pub proxy: AdapterProxyConfig,      // HTTP 代理配置
}
```

`from_platform_config` 方法按以下优先级读取：

1. **token**：优先 `platforms.weixin.token`，回退到 `extra.token`。
2. **account_id**：从 `extra.account_id` 读取。
3. **其他字段**：从 `extra` 中对应键名读取，空值则使用默认值。
4. **allow_from / group_allow_from**：支持 JSON 数组或逗号分隔字符串两种格式。

### 环境变量覆盖机制

环境变量可以覆盖 config.yaml 中的配置。优先级：环境变量 > config.yaml > 账号文件 > 默认值。

| 环境变量 | 对应字段 | 说明 |
|----------|----------|------|
| `WEIXIN_ACCOUNT_ID` | `extra.account_id` | iLink Bot 账号 ID |
| `WEIXIN_TOKEN` | `token` / `extra.token` | iLink Bot 令牌 |
| `WEIXIN_BASE_URL` | `extra.base_url` | iLink API 基础 URL |
| `WEIXIN_CDN_BASE_URL` | `extra.cdn_base_url` | CDN 基础 URL |
| `WEIXIN_DM_POLICY` | `extra.dm_policy` | DM 访问策略 |
| `WEIXIN_GROUP_POLICY` | `extra.group_policy` | 群组访问策略 |
| `WEIXIN_ALLOWED_USERS` | `extra.allow_from` | DM 白名单（逗号分隔） |
| `WEIXIN_GROUP_ALLOWED_USERS` | `extra.group_allow_from` | 群组白名单（逗号分隔，注意：虽然变量名含 USERS，但实际期望的是群组 ID） |
| `WEIXIN_HOME_CHANNEL` | `home_channel` | Home channel chat ID |
| `WEIXIN_HOME_CHANNEL_NAME` | -- | Home channel 显示名称（默认 `Home`） |
| `WEIXIN_SPLIT_MULTILINE_MESSAGES` | -- | 是否拆分多行消息（旧版行为） |
| `WEIXIN_ALLOW_ALL_USERS` | -- | Gateway 级别允许所有用户（向导使用） |
| `HERMES_WEIXIN_QR_LOGIN` | -- | 设为 truthy 值启用 QR 登录模式 |

DM 策略的读取优先级（`platform_dm_policy` 函数）：

1. `extra.dm_policy`
2. `extra.extra.dm_policy`（嵌套情况）
3. 环境变量 `WEIXIN_DM_POLICY`
4. 默认值 `"open"`（Weixin 特有的默认值，大多数其他平台默认为 `pairing`）

### 凭证文件路径和格式

所有 Weixin 凭证文件存储在 `~/.hermes/weixin/accounts/` 目录下：

| 文件 | 路径模式 | 说明 |
|------|----------|------|
| 账号凭证 | `<account_id>.json` | token、base_url、user_id、saved_at |
| 同步游标 | `<account_id>.sync.json` | `get_updates_buf` 长轮询游标 |
| Context Token | `<account_id>.context-tokens.json` | 每个对话 peer 的 context_token 映射 |

账号凭证文件格式：

```json
{
  "token": "ilink-bot-token-string",
  "base_url": "https://ilinkai.weixin.qq.com",
  "user_id": "ilink-user-id",
  "saved_at": "2026-06-01T12:00:00Z"
}
```

---

## 4. DM 访问策略详解

### 四种策略的行为差异

| 策略 | 对应 `DmAccessMode` | 行为 |
|------|---------------------|------|
| `open` | `Open` | 允许任何人发送私聊消息，跳过 DmManager 检查 |
| `pairing` | `Pairing` | 未授权用户发送消息时，触发配对审批流程 |
| `allowlist` | `Allowlist` | 仅 `allow_from` 列表中的用户可以发送私聊消息 |
| `disabled` | `Disabled` | 所有私聊消息被静默丢弃 |

Weixin 平台默认 DM 策略为 `open`（不同于大多数其他平台的默认值 `pairing`）。

### 配对码（Pairing）机制

#### 什么是配对码

配对码是 Hermes Gateway 的 DM 授权机制，用于控制未注册用户与 bot 的交互。当 `dm_policy` 设为 `pairing` 时，未授权用户的第一条消息不会触发 Agent 处理，而是返回一条审批提示消息。

#### 配对流程

```
用户 → [发送任意消息] → Gateway
                          ↓
                    DmManager.handle_dm()
                          ↓
                    检查 allow_all_dm?  → 是 → Allow
                    检查 admin_users?   → 是 → Allow
                    检查 authorized_users? → 是 → Allow
                          ↓ 否
                    UnauthorizedDmBehavior::Pair
                          ↓
                    返回 DmDecision::Pair
                          ↓
                    发送审批提示消息给用户
                    消息不进入 Agent 处理
```

用户收到的审批提示消息内容：

```
Your request has been submitted for approval. You will be notified once an admin reviews it.
```

此时消息被拦截，不会传递给 Agent 处理。用户需要等待管理员审批后才能正常与 bot 交互。

#### 配对状态管理

`DmManager` 内部维护两个集合：

- `authorized_users: HashSet<String>` -- 已授权的普通用户
- `admin_users: HashSet<String>` -- 管理员用户（始终允许）

授权/撤销操作通过 `DmManager` 的方法进行：

```rust
dm_manager.authorize_user("user_id");     // 添加授权用户
dm_manager.deauthorize_user("user_id");   // 撤销授权用户
dm_manager.add_admin("admin_id");          // 添加管理员
dm_manager.remove_admin("admin_id");       // 撤销管理员
```

#### 推荐配置

对于个人使用：使用 `open`（默认值），允许所有私聊。

对于团队/组织部署：使用 `allowlist`，预配置允许的用户 ID。

对于需要审批的场景：使用 `pairing`，配合管理员审批流程。

### allowlist 模式

当 `dm_policy` 设为 `allowlist` 时，只有 `allow_from` 列表中列出的用户 ID 才能与 bot 交互。

配置方式（config.yaml）：

```yaml
platforms:
  weixin:
    extra:
      dm_policy: "allowlist"
      allow_from:
        - "user_abc123"
        - "user_def456"
```

配置方式（环境变量）：

```bash
WEIXIN_DM_POLICY=allowlist
WEIXIN_ALLOWED_USERS=user_abc123,user_def456
```

`allow_from` 支持两种输入格式：
- JSON 数组：`["user1", "user2"]`
- 逗号分隔字符串：`"user1,user2"`

用户 ID 匹配支持大小写不敏感比较，并自动去除 `@` 前缀。

---

## 5. 群组策略

### 三种策略的区别

| 策略 | 对应 `GroupAccessMode` | 行为 |
|------|----------------------|------|
| `disabled`（默认） | `Disabled` | 所有群组消息被静默丢弃 |
| `open` | `Open` | 在所有群组中响应（前提是 iLink 转发了群组事件） |
| `allowlist` | `Allowlist` | 仅响应 `group_allow_from` 列表中的群组 |

Weixin 的群组策略默认值为 `disabled`（不同于 WeCom 的默认值 `open`）。这是因为：
- 个人微信账号可能加入了许多群组。
- iLink bot 身份通常无法接收普通微信群聊消息。

如果在启动时将 `WEIXIN_GROUP_POLICY` 设为非 `disabled` 值，Gateway 会输出一条 WARNING 日志。

### 群组识别方式

群组通过 chat ID 的后缀 `@chatroom` 来识别：

```rust
fn get_chat_type(chat_id: &str) -> &'static str {
    if chat_id.ends_with("@chatroom") {
        "group"
    } else {
        "dm"
    }
}
```

- chat ID 以 `@chatroom` 结尾 → 群组消息
- 其他 → 私聊消息

### 群组白名单配置

```yaml
platforms:
  weixin:
    extra:
      group_policy: "allowlist"
      group_allow_from:
        - "12345678@chatroom"
        - "87654321@chatroom"
```

环境变量方式：

```bash
WEIXIN_GROUP_POLICY=allowlist
WEIXIN_GROUP_ALLOWED_USERS=12345678@chatroom,87654321@chatroom
```

注意：`WEIXIN_GROUP_ALLOWED_USERS` 变量名包含 "USERS"，但实际期望填入的是**群组 chat ID**，而非用户 ID。这是历史遗留命名。

---

## 6. 启动与验证

### 启动命令

**交互式配置后启动**：

```bash
hermes gateway setup    # 首次配置
hermes gateway          # 启动 Gateway
```

**直接使用 auth 命令登录**：

```bash
hermes auth login weixin --qr    # QR 码登录
hermes gateway                   # 启动 Gateway
```

### 启动流程

Gateway 启动时，Weixin 适配器的初始化流程如下：

1. 检查 `platforms.weixin.enabled` 是否为 `true`。
2. 检查 `extra.account_id` 是否存在且非空。
3. 检查 token 是否存在（优先 `platforms.weixin.token`，回退到 `extra.token`）。
4. 如果 token 为空，尝试从 `~/.hermes/weixin/accounts/<account_id>.json` 加载持久化 token。
5. 如果 base_url 为默认值，尝试从持久化账号文件加载。
6. 验证 token 格式。
7. 构建 HTTP 客户端。
8. 注册适配器并启动长轮询循环。

### 正常日志示例

启动成功后，Gateway 开始长轮询：

```
Weixin adapter registered, starting long-poll loop...
```

收到消息时的调试日志：

```
DEBUG weixin inbound: user=<user_id> chat=<chat_id> text_len=42
```

### 调试日志配置

启用调试日志以获得更详细的 Weixin 适配器输出：

```bash
RUST_LOG=hermes_gateway=debug hermes gateway
```

或仅针对 Weixin 模块：

```bash
RUST_LOG=hermes_gateway::platforms::weixin=debug hermes gateway
```

---

## 7. 常见问题排查

### 问题与解决方案

| 问题 | 原因 | 解决方案 |
|------|------|----------|
| `Weixin is enabled but account_id is missing` | config.yaml 中缺少 account_id | 运行 `hermes auth login weixin --qr` 或手动设置 `platforms.weixin.extra.account_id` |
| `Weixin is enabled but token is missing` | 无可用 token | 运行 `hermes auth login weixin --qr` 或设置 `platforms.weixin.token` |
| `Weixin iLink requires account_id (WEIXIN_ACCOUNT_ID)` | 初始化时 account_id 为空 | 在 config.yaml 或环境变量中设置 |
| `Weixin iLink requires token (WEIXIN_TOKEN or saved account file)` | token 和持久化文件均不存在 | 重新执行 QR 登录或手动设置 token |
| `Another local Hermes gateway is already using this Weixin token` | Token 锁冲突 | 停止另一个 Gateway 实例 -- 同一 token 只允许一个轮询器 |
| 会话过期（`errcode=-14`） | iLink 登录会话过期 | 重新运行 `hermes auth login weixin --qr` 扫描新 QR 码 |
| `weixin qr expired too many times` | QR 码连续过期超过 3 次 | 检查网络连接后重试 |
| Bot 不回复私聊 | DM 策略限制 | 检查 `WEIXIN_DM_POLICY`，如设为 `allowlist` 则确认发送者在白名单中 |
| Bot 忽略群组消息 | 群组策略默认 disabled | 设置 `WEIXIN_GROUP_POLICY=open` 或 `allowlist`。但注意 QR 登录的 iLink bot 身份通常无法接收群组消息 |
| 媒体上传/下载失败 | 缺少依赖或网络问题 | 确保可访问 `novac2c.cdn.weixin.qq.com`，检查网络连接 |
| `weixin SSRF: host 'xxx' not in CDN allowlist` | 媒体 URL 不在允许的域名列表中 | 仅允许访问白名单域名（见 SSRF 防护章节） |
| 语音消息显示为文本 | 微信提供了转写文本 | 正常行为 -- 适配器优先使用转写文本 |
| 消息重复 | 多个 Gateway 实例运行 | 确保只运行一个实例，适配器通过消息 ID 去重 |
| `iLink POST ... HTTP 4xx/5xx` | iLink API 错误 | 检查 token 有效性和网络连通性 |
| 终端 QR 码不显示 | 终端不支持 Unicode 块字符 | 使用终端输出的 URL 代替，或安装 messaging extra |

### TLS close_notify 日志说明

在长轮询过程中，可能会看到类似以下的 TLS close_notify 日志：

```
TLS close_notify received during long-poll
```

这是**正常行为**。iLink API 服务端在长时间无消息时会关闭 TLS 连接，客户端收到 close_notify 后会自动重新发起轮询请求。这不是错误，无需处理。

### 重试策略

| 条件 | 行为 |
|------|------|
| 瞬时错误（第 1-2 次） | 2 秒后重试 |
| 连续错误（3 次以上） | 退避 30 秒，然后重置计数器 |
| 会话过期（`errcode=-14`） | 暂停 10 分钟（可能需要重新登录） |
| 超时 | 立即重新轮询（正常长轮询行为） |
| 速率限制（`errcode=-2` 且 `errmsg="unknown error"`） | 判定为过期会话而非真正限速 |

---

## 8. 技术细节

### AES-128-ECB 媒体加密机制

微信媒体文件通过加密 CDN 传输，适配器自动处理加解密：

**入站（接收）**：

1. 从 CDN 下载加密文件（使用 `encrypted_query_param` 构建下载 URL）。
2. 使用消息中提供的 `aes_key`（base64 编码）解密。
3. AES-128-ECB 模式，PKCS#7 填充。
4. 解密后的明文数据缓存到本地供 Agent 处理。

**出站（发送）**：

1. 生成随机 AES-128 密钥（16 字节）。
2. 使用 AES-128-ECB + PKCS#7 填充加密文件。
3. 向 iLink API 请求上传 URL（`ilink/bot/getuploadurl`）。
4. 上传密文到 CDN。
5. 发送消息时携带加密媒体引用。

**AES 密钥格式处理**：

密钥以 base64 编码传入，解码后可能是两种格式：
- 16 字节原始密钥 -- 直接使用。
- 32 字节十六进制字符串 -- 按 hex 解码为 16 字节。

相关函数：

```rust
fn parse_aes_key(aes_key_b64: &str) -> Result<[u8; 16], GatewayError>
fn aes128_ecb_encrypt(plaintext: &[u8], key_bytes: &[u8; 16]) -> Vec<u8>
fn aes128_ecb_decrypt(ciphertext: &[u8], key_bytes: &[u8; 16]) -> Result<Vec<u8>, GatewayError>
```

CDN 允许域名列表（`WEIXIN_CDN_ALLOWLIST`）：

```
novac2c.cdn.weixin.qq.com
ilinkai.weixin.qq.com
wx.qlogo.cn
thirdwx.qlogo.cn
res.wx.qq.com
mmbiz.qpic.cn
mmbiz.qlogo.cn
```

### Context Token 机制

iLink Bot API 要求每条出站消息携带对应 peer 的 `context_token`，以确保回复连续性。

**工作机制**：

1. **入站**：每条收到的消息中携带 `context_token`，适配器将其保存到内存和磁盘。
2. **出站**：发送消息时自动附加该 peer 最新的 context_token。
3. **持久化**：Token 按 `account_id + peer_id` 映射存储在 `~/.hermes/weixin/accounts/<account_id>.context-tokens.json`。
4. **恢复**：Gateway 重启时从磁盘恢复 context_token，确保回复不中断。

### 消息去重

使用消息 ID 进行去重，防止网络抖动或重叠轮询响应导致重复处理：

- **去重窗口**：5 分钟（`DEDUP_TTL = Duration::from_secs(300)`）。
- **存储**：内存 `HashMap<String, Instant>`，键为消息 ID，值为首次见到的时间戳。
- **清理**：每次检查时自动清理过期条目。

实现位置：

```rust
seen: Mutex<HashMap<String, Instant>>
```

### SSRF 防护

所有出站媒体 URL 在下载前必须通过域名白名单验证：

```rust
fn assert_weixin_cdn_url(url: &str) -> Result<(), GatewayError> {
    // 解析 URL 并检查 host 是否在 WEIXIN_CDN_ALLOWLIST 中
    // 不在白名单中则返回:
    // GatewayError::Platform("weixin SSRF: host 'xxx' not in CDN allowlist")
}
```

该机制防止恶意消息中包含指向内部网络或私有地址的 URL，避免 SSRF（Server-Side Request Forgery）攻击。

### Token 锁

同一时间只允许一个 Gateway 实例使用特定 token。适配器在启动时获取 token 锁，关闭时释放。如果另一个 Gateway 已在使用同一 token，启动将失败并显示明确的错误信息。

### 输入指示器（Typing Indicator）

适配器在 Agent 处理消息时向微信客户端显示"正在输入"状态：

1. 收到消息时，通过 `getconfig` API 获取 `typing_ticket`。
2. Typing ticket 按用户缓存，TTL 为 10 分钟。
3. `sendtyping` 端点使用 `TYPING_START(1)` 和 `TYPING_STOP(2)` 控制状态。
4. Gateway 自动在 Agent 处理消息期间触发输入指示器。

### 消息分块

消息在不超过平台限制时作为单条发送，超大消息按逻辑边界拆分：

- 最大消息长度：**4000 字符**（代码中 `MAX_TEXT = 2000` 用于入站文本截断参考）。
- 未超限的消息保持完整，即使包含多段落或换行。
- 超大消息在段落、空行、代码围栏处拆分。
- 代码围栏块尽量保持完整（除非单个块本身超限）。
- 分块间有 0.3 秒延迟以防止微信限速。
