use std::sync::Arc;

use hermes_config::GatewayConfig;

use super::super::App;
use super::super::traits::ModelRuntime;

impl ModelRuntime for App {
    fn config(&self) -> &Arc<GatewayConfig> {
        &self.core.config
    }

    fn set_config(&mut self, config: Arc<GatewayConfig>) {
        self.core.config = config;
    }

    fn current_model(&self) -> &str {
        &self.model.current_model
    }

    fn current_model_mut(&mut self) -> &mut String {
        &mut self.model.current_model
    }

    fn current_personality(&self) -> Option<&str> {
        self.model.current_personality.as_deref()
    }

    fn switch_model(&mut self, provider_model: &str) {
        self.model.switch_active(
            provider_model,
            &mut self.core,
            &self.session,
            &self.state_root,
            &self.stream,
        );
    }

    fn switch_personality(&mut self, name: &str) {
        self.model.switch_personality_name(name);
    }

    fn current_runtime_provider(&self) -> String {
        App::current_runtime_provider(self)
    }
}
