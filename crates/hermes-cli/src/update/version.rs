use semver::Version;
use serde::{Deserialize, Serialize};

/// 发布渠道
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Default)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    #[default]
    Stable,
    Rc,
    Beta,
    Nightly,
}

impl Channel {
    /// 从 pre-release 字符串推导渠道
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
            tracing::warn!(
                "Unknown pre-release suffix '{}', defaulting to Beta channel",
                pre
            );
            Channel::Beta // 未知 pre-release 默认视为 beta
        }
    }

    /// 从字符串解析渠道
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "stable" => Channel::Stable,
            "rc" => Channel::Rc,
            "beta" => Channel::Beta,
            "nightly" => Channel::Nightly,
            _ => Channel::Stable,
        }
    }
}

/// 版本比较决策
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateDecision {
    /// 已是最新
    UpToDate,
    /// 有可用更新
    UpdateAvailable { forced: bool },
    /// 不应更新（降级或其它原因）
    DoNotUpdate { reason: String },
}

/// 版本比较策略 trait（Strategy 模式）
pub trait VersionPolicy: Send + Sync {
    fn evaluate(&self, current: &Version, available: &Version, meta: &UpdateMeta)
    -> UpdateDecision;

    fn name(&self) -> &str;
}

/// 来自服务器的更新元数据
#[derive(Debug, Clone, Default)]
pub struct UpdateMeta {
    pub channel: Channel,
    pub forced: bool,
    pub min_supported_version: Option<Version>,
    pub deprecated_versions: Vec<Version>,
}

/// 严格 SemVer 比较策略（默认）
/// - available > current → 更新
/// - available <= current → 不更新
/// - forced=true 时忽略版本比较，总是建议更新
/// - 检查 min_supported_version，低于最低版本时强制更新
pub struct SemverPolicy;

impl VersionPolicy for SemverPolicy {
    fn evaluate(
        &self,
        current: &Version,
        available: &Version,
        meta: &UpdateMeta,
    ) -> UpdateDecision {
        // 检查是否被废弃
        if meta.deprecated_versions.contains(available) {
            return UpdateDecision::DoNotUpdate {
                reason: format!("Version {} is deprecated", available),
            };
        }

        // 检查最低支持版本
        if let Some(ref min) = meta.min_supported_version {
            if current < min {
                return UpdateDecision::UpdateAvailable { forced: true };
            }
        }

        // 强制更新优先
        if meta.forced {
            if available != current {
                return UpdateDecision::UpdateAvailable { forced: true };
            }
        }

        // 标准 SemVer 比较
        match available.cmp(current) {
            std::cmp::Ordering::Greater => UpdateDecision::UpdateAvailable { forced: false },
            std::cmp::Ordering::Equal => UpdateDecision::UpToDate,
            std::cmp::Ordering::Less => UpdateDecision::DoNotUpdate {
                reason: format!(
                    "Available version {} is older than current {}",
                    available, current
                ),
            },
        }
    }

    fn name(&self) -> &str {
        "semver"
    }
}

/// 渠道感知策略
/// - stable 用户不推送 pre-release 版本
/// - beta 用户可收到 rc 和 stable
/// - nightly 用户收到所有版本
pub struct ChannelPolicy {
    /// 用户当前订阅的渠道
    pub subscribed_channel: Channel,
}

impl VersionPolicy for ChannelPolicy {
    fn evaluate(
        &self,
        current: &Version,
        available: &Version,
        meta: &UpdateMeta,
    ) -> UpdateDecision {
        // 强制更新总是推送
        if meta.forced && available != current {
            return UpdateDecision::UpdateAvailable { forced: true };
        }

        // 渠道过滤：stable 用户不应收到 pre-release
        let available_channel = Channel::from_prerelease(&available.pre.to_string());

        match self.subscribed_channel {
            Channel::Stable => {
                if available_channel != Channel::Stable {
                    return UpdateDecision::DoNotUpdate {
                        reason: format!(
                            "Pre-release version {} not pushed to stable channel",
                            available
                        ),
                    };
                }
            }
            Channel::Rc => {
                if available_channel == Channel::Nightly || available_channel == Channel::Beta {
                    return UpdateDecision::DoNotUpdate {
                        reason: format!(
                            "Channel {:?} not pushed to rc subscribers",
                            available_channel
                        ),
                    };
                }
            }
            Channel::Beta => {
                if available_channel == Channel::Nightly {
                    return UpdateDecision::DoNotUpdate {
                        reason: "Nightly not pushed to beta subscribers".to_string(),
                    };
                }
            }
            Channel::Nightly => {
                // Nightly 用户接收所有
            }
        }

        // 通过渠道过滤后，做版本比较
        match available.cmp(current) {
            std::cmp::Ordering::Greater => UpdateDecision::UpdateAvailable { forced: false },
            std::cmp::Ordering::Equal => UpdateDecision::UpToDate,
            std::cmp::Ordering::Less => UpdateDecision::DoNotUpdate {
                reason: format!("Available {} is older than current {}", available, current),
            },
        }
    }

    fn name(&self) -> &str {
        "channel-aware"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use semver::Version;

    fn default_meta() -> UpdateMeta {
        UpdateMeta::default()
    }

    #[test]
    fn test_semver_upgrade() {
        let policy = SemverPolicy;
        let current = Version::parse("1.0.0").unwrap();
        let available = Version::parse("1.1.0").unwrap();
        let decision = policy.evaluate(&current, &available, &default_meta());
        assert_eq!(decision, UpdateDecision::UpdateAvailable { forced: false });
    }

    #[test]
    fn test_semver_same() {
        let policy = SemverPolicy;
        let current = Version::parse("1.0.0").unwrap();
        let available = Version::parse("1.0.0").unwrap();
        let decision = policy.evaluate(&current, &available, &default_meta());
        assert_eq!(decision, UpdateDecision::UpToDate);
    }

    #[test]
    fn test_semver_downgrade() {
        let policy = SemverPolicy;
        let current = Version::parse("2.0.0").unwrap();
        let available = Version::parse("1.0.0").unwrap();
        let decision = policy.evaluate(&current, &available, &default_meta());
        assert!(matches!(decision, UpdateDecision::DoNotUpdate { .. }));
    }

    #[test]
    fn test_semver_prerelease() {
        // 1.0.0-beta.1 < 1.0.0 per SemVer spec
        let pre = Version::parse("1.0.0-beta.1").unwrap();
        let stable = Version::parse("1.0.0").unwrap();
        assert!(pre < stable);

        // Policy: upgrading from beta to stable should be UpdateAvailable
        let policy = SemverPolicy;
        let decision = policy.evaluate(&pre, &stable, &default_meta());
        assert_eq!(decision, UpdateDecision::UpdateAvailable { forced: false });
    }

    #[test]
    fn test_forced_override() {
        let policy = SemverPolicy;
        let current = Version::parse("1.0.0").unwrap();
        let available = Version::parse("1.0.1").unwrap();
        let meta = UpdateMeta {
            forced: true,
            ..Default::default()
        };
        let decision = policy.evaluate(&current, &available, &meta);
        assert_eq!(decision, UpdateDecision::UpdateAvailable { forced: true });
    }

    #[test]
    fn test_min_version_forces_update() {
        let policy = SemverPolicy;
        let current = Version::parse("0.9.0").unwrap();
        let available = Version::parse("1.0.0").unwrap();
        let meta = UpdateMeta {
            min_supported_version: Some(Version::parse("1.0.0").unwrap()),
            ..Default::default()
        };
        let decision = policy.evaluate(&current, &available, &meta);
        assert_eq!(decision, UpdateDecision::UpdateAvailable { forced: true });
    }

    #[test]
    fn test_deprecated_version() {
        let policy = SemverPolicy;
        let current = Version::parse("1.0.0").unwrap();
        let available = Version::parse("1.1.0").unwrap();
        let meta = UpdateMeta {
            deprecated_versions: vec![Version::parse("1.1.0").unwrap()],
            ..Default::default()
        };
        let decision = policy.evaluate(&current, &available, &meta);
        assert!(matches!(decision, UpdateDecision::DoNotUpdate { .. }));
    }

    #[test]
    fn test_channel_stable_no_prerelease() {
        let policy = ChannelPolicy {
            subscribed_channel: Channel::Stable,
        };
        let current = Version::parse("1.0.0").unwrap();
        let available = Version::parse("1.1.0-beta.1").unwrap();
        let decision = policy.evaluate(&current, &available, &default_meta());
        assert!(matches!(decision, UpdateDecision::DoNotUpdate { .. }));
    }

    #[test]
    fn test_channel_nightly_all() {
        let policy = ChannelPolicy {
            subscribed_channel: Channel::Nightly,
        };
        let current = Version::parse("1.0.0").unwrap();
        let available = Version::parse("1.1.0-nightly.42").unwrap();
        let decision = policy.evaluate(&current, &available, &default_meta());
        assert_eq!(decision, UpdateDecision::UpdateAvailable { forced: false });
    }

    #[test]
    fn test_channel_from_prerelease() {
        assert_eq!(Channel::from_prerelease(""), Channel::Stable);
        assert_eq!(Channel::from_prerelease("beta.1"), Channel::Beta);
        assert_eq!(Channel::from_prerelease("rc.1"), Channel::Rc);
        assert_eq!(
            Channel::from_prerelease("nightly.20240101"),
            Channel::Nightly
        );
        assert_eq!(Channel::from_prerelease("alpha.1"), Channel::Beta); // unknown → beta
    }

    #[test]
    fn test_channel_from_str() {
        assert_eq!(Channel::from_str("stable"), Channel::Stable);
        assert_eq!(Channel::from_str("rc"), Channel::Rc);
        assert_eq!(Channel::from_str("beta"), Channel::Beta);
        assert_eq!(Channel::from_str("nightly"), Channel::Nightly);
        assert_eq!(Channel::from_str("unknown"), Channel::Stable); // default
    }
}
