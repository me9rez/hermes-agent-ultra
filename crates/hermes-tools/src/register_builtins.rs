//! Entry-point shims for registering all built-in tool handlers.
//!
//! Each public function builds a [`register::RegistryContext`] from its
//! arguments and delegates to [`register::register_all`], which fans out to
//! the per-family sub-modules under `register/`.

use std::sync::Arc;

use hermes_config::voice::{SttConfig, TtsConfig};
use hermes_core::{SkillProvider, TerminalBackend};

use crate::ToolRegistry;
use crate::register::{RegistryContext, register_all};

/// Voice/media config passed into built-in TTS/STT tools.
#[derive(Debug, Clone, Default)]
pub struct VoiceMediaToolConfig {
    pub tts: Option<TtsConfig>,
    pub stt: Option<SttConfig>,
}

/// Register built-in tools without an injected vision backend.
pub fn register_builtin_tools(
    registry: &ToolRegistry,
    terminal_backend: Arc<dyn TerminalBackend>,
    skill_provider: Arc<dyn SkillProvider>,
) {
    register_builtin_tools_impl(registry, terminal_backend, skill_provider, None, None);
}

/// Register built-in tools with optional voice (tts/stt) config from `GatewayConfig`.
pub fn register_builtin_tools_with_voice(
    registry: &ToolRegistry,
    terminal_backend: Arc<dyn TerminalBackend>,
    skill_provider: Arc<dyn SkillProvider>,
    voice: Option<VoiceMediaToolConfig>,
) {
    register_builtin_tools_impl(registry, terminal_backend, skill_provider, None, voice);
}

/// Register all built-in tool handlers into the given registry.
///
/// `vision_backend` should be an [`AuxiliaryVisionAdapter`] when auxiliary LLM is configured.
pub fn register_builtin_tools_with_vision(
    registry: &ToolRegistry,
    terminal_backend: Arc<dyn TerminalBackend>,
    skill_provider: Arc<dyn SkillProvider>,
    vision_backend: Option<Arc<dyn crate::tools::vision::VisionBackend>>,
) {
    register_builtin_tools_impl(
        registry,
        terminal_backend,
        skill_provider,
        vision_backend,
        None,
    );
}

pub fn register_builtin_tools_with_vision_and_voice(
    registry: &ToolRegistry,
    terminal_backend: Arc<dyn TerminalBackend>,
    skill_provider: Arc<dyn SkillProvider>,
    vision_backend: Option<Arc<dyn crate::tools::vision::VisionBackend>>,
    voice: Option<VoiceMediaToolConfig>,
) {
    register_builtin_tools_impl(
        registry,
        terminal_backend,
        skill_provider,
        vision_backend,
        voice,
    );
}

fn register_builtin_tools_impl(
    registry: &ToolRegistry,
    terminal_backend: Arc<dyn TerminalBackend>,
    skill_provider: Arc<dyn SkillProvider>,
    vision_backend: Option<Arc<dyn crate::tools::vision::VisionBackend>>,
    voice: Option<VoiceMediaToolConfig>,
) {
    let tts_cfg = voice.as_ref().and_then(|v| v.tts.clone());
    let stt_cfg = voice.as_ref().and_then(|v| v.stt.clone());
    let terminal_check: Arc<dyn Fn() -> bool + Send + Sync> =
        Arc::new(crate::terminal_requirements::check_terminal_requirements);

    let ctx = RegistryContext {
        registry,
        terminal_backend,
        skill_provider,
        vision_backend,
        tts_cfg,
        stt_cfg,
        terminal_check,
    };
    register_all(&ctx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use hermes_core::{AgentError, CommandOutput, Skill, SkillMeta};

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = std::env::var(key).ok();
            unsafe { std::env::set_var(key, value) };
            Self { key, original }
        }

        fn remove(key: &'static str) -> Self {
            let original = std::env::var(key).ok();
            unsafe { std::env::remove_var(key) };
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                if let Some(value) = &self.original {
                    std::env::set_var(self.key, value);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    struct MockTerminalBackend;

    #[async_trait]
    impl TerminalBackend for MockTerminalBackend {
        async fn execute_command(
            &self,
            _command: &str,
            _timeout: Option<u64>,
            _workdir: Option<&str>,
            _background: bool,
            _pty: bool,
        ) -> Result<CommandOutput, AgentError> {
            Ok(CommandOutput {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
            })
        }

        async fn read_file(
            &self,
            _path: &str,
            _offset: Option<u64>,
            _limit: Option<u64>,
        ) -> Result<String, AgentError> {
            Ok(String::new())
        }

        async fn write_file(&self, _path: &str, _content: &str) -> Result<(), AgentError> {
            Ok(())
        }

        async fn file_exists(&self, _path: &str) -> Result<bool, AgentError> {
            Ok(false)
        }

        async fn list_processes(&self) -> Result<serde_json::Value, AgentError> {
            Ok(serde_json::json!([]))
        }
    }

    struct MockSkillProvider;

    #[async_trait]
    impl SkillProvider for MockSkillProvider {
        async fn create_skill(
            &self,
            name: &str,
            content: &str,
            category: Option<&str>,
        ) -> Result<Skill, AgentError> {
            Ok(Skill {
                name: name.into(),
                content: content.into(),
                category: category.map(String::from),
                description: None,
            })
        }

        async fn get_skill(&self, _name: &str) -> Result<Option<Skill>, AgentError> {
            Ok(None)
        }

        async fn list_skills(&self) -> Result<Vec<SkillMeta>, AgentError> {
            Ok(Vec::new())
        }

        async fn update_skill(&self, name: &str, content: &str) -> Result<Skill, AgentError> {
            Ok(Skill {
                name: name.into(),
                content: content.into(),
                category: None,
                description: None,
            })
        }

        async fn delete_skill(&self, _name: &str) -> Result<(), AgentError> {
            Ok(())
        }
    }

    fn registered_names() -> Vec<String> {
        let registry = ToolRegistry::new();
        register_builtin_tools(
            &registry,
            Arc::new(MockTerminalBackend),
            Arc::new(MockSkillProvider),
        );
        let mut names: Vec<String> = registry
            .list_tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        names.sort();
        names
    }

    #[test]
    fn local_backend_exposes_terminal_and_terminal_backed_file_tools() {
        let _lock = lock_env();
        let home = tempfile::tempdir().expect("temp home");
        let _home = EnvGuard::set("HOME", home.path().to_string_lossy().as_ref());
        let _terminal_env = EnvGuard::set("TERMINAL_ENV", "local");
        let names = registered_names();

        for expected in [
            "terminal",
            "process",
            "process_registry",
            "read_file",
            "write_file",
            "patch",
            "search_files",
        ] {
            assert!(
                names.contains(&expected.to_string()),
                "local backend should expose {expected}"
            );
        }
    }

    #[test]
    fn fal_media_tools_hidden_without_credentials() {
        let _lock = lock_env();
        let home = tempfile::tempdir().expect("temp home");
        let _home = EnvGuard::set("HOME", home.path().to_string_lossy().as_ref());
        let _terminal_env = EnvGuard::set("TERMINAL_ENV", "local");
        let _fal = EnvGuard::remove("FAL_KEY");
        let _managed = EnvGuard::remove("HERMES_ENABLE_NOUS_MANAGED_TOOLS");
        let _token = EnvGuard::remove("TOOL_GATEWAY_USER_TOKEN");
        let registry = ToolRegistry::new();
        register_builtin_tools(
            &registry,
            Arc::new(MockTerminalBackend),
            Arc::new(MockSkillProvider),
        );
        let exposed: Vec<String> = registry
            .get_definitions()
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert!(
            !exposed.contains(&"image_generate".to_string()),
            "image_generate should not be exposed without FAL credentials"
        );
        assert!(
            !exposed.contains(&"video_generate".to_string()),
            "video_generate should not be exposed without FAL credentials"
        );
        let registered: Vec<String> = registry.list_tools().into_iter().map(|t| t.name).collect();
        assert!(registered.contains(&"image_generate".to_string()));
        assert!(registered.contains(&"video_generate".to_string()));
    }

    #[test]
    fn invalid_backend_hides_terminal_backed_tools_but_keeps_local_file_tools() {
        let _lock = lock_env();
        let home = tempfile::tempdir().expect("temp home");
        let _home = EnvGuard::set("HOME", home.path().to_string_lossy().as_ref());
        let _terminal_env = EnvGuard::set("TERMINAL_ENV", "unknown-backend");
        let _ssh_host = EnvGuard::remove("TERMINAL_SSH_HOST");
        let _ssh_user = EnvGuard::remove("TERMINAL_SSH_USER");
        let names = registered_names();

        for hidden in [
            "terminal",
            "process",
            "process_registry",
            "read_file",
            "write_file",
        ] {
            assert!(
                !names.contains(&hidden.to_string()),
                "invalid backend should hide {hidden}"
            );
        }
        for independent in ["patch", "search_files", "execute_code"] {
            assert!(
                names.contains(&independent.to_string()),
                "invalid backend should keep independent tool {independent}"
            );
        }
    }
}
