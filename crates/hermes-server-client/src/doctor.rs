//! Connectivity and configuration diagnostics for the remote LLM server.

use hermes_config::ServerConfig;

use crate::auth::AuthManager;
use crate::session::TokenSource;
use crate::transport::HttpTransport;

#[derive(Debug, Clone)]
pub struct DoctorReport {
    pub checks: Vec<DoctorCheck>,
}

#[derive(Debug, Clone)]
pub struct DoctorCheck {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

impl DoctorReport {
    pub fn all_ok(&self) -> bool {
        self.checks.iter().all(|c| c.ok)
    }

    pub fn print_lines(&self) -> Vec<String> {
        self.checks
            .iter()
            .map(|c| {
                let mark = if c.ok { "ok" } else { "FAIL" };
                format!("[{mark}] {} — {}", c.name, c.detail)
            })
            .collect()
    }
}

pub async fn run_doctor(
    config: &ServerConfig,
    hermes_home: impl AsRef<std::path::Path>,
) -> DoctorReport {
    let mut checks = Vec::new();

    checks.push(DoctorCheck {
        name: "server.enabled",
        ok: config.enabled,
        detail: if config.enabled {
            "server integration enabled".to_string()
        } else {
            "server integration disabled (set server.enabled or HERMES_SERVER_ENABLED)".to_string()
        },
    });

    let base_ok = !config.base_url.trim().is_empty();
    checks.push(DoctorCheck {
        name: "server.base_url",
        ok: !config.enabled || base_ok,
        detail: if base_ok {
            config.base_url.clone()
        } else {
            "base_url empty — set server.base_url or HERMES_SERVER_URL".to_string()
        },
    });

    let manager_result = AuthManager::new(config.clone(), &hermes_home);
    match manager_result {
        Ok(manager) => {
            match manager.whoami().await {
                Ok(status) => {
                    checks.push(DoctorCheck {
                        name: "auth.token",
                        ok: !config.enabled || status.is_logged_in(),
                        detail: if status.is_logged_in() {
                            format!(
                                "logged in via {} {}",
                                status.source,
                                if status.token_expired() {
                                    "(token expired)"
                                } else {
                                    ""
                                }
                            )
                        } else {
                            format!("not logged in ({})", status.source)
                        },
                    });
                }
                Err(err) => {
                    checks.push(DoctorCheck {
                        name: "auth.token",
                        ok: false,
                        detail: err.to_string(),
                    });
                }
            }

            if config.enabled && base_ok {
                let transport = HttpTransport::new(config);
                match transport {
                    Ok(t) => match t.get("/health", None).await {
                        Ok(resp) => {
                            let ok = resp.status().is_success();
                            checks.push(DoctorCheck {
                                name: "server.health",
                                ok,
                                detail: format!("GET /health -> HTTP {}", resp.status()),
                            });
                        }
                        Err(err) => {
                            checks.push(DoctorCheck {
                                name: "server.health",
                                ok: false,
                                detail: format!("GET /health failed: {err}"),
                            });
                        }
                    },
                    Err(err) => {
                        checks.push(DoctorCheck {
                            name: "server.health",
                            ok: false,
                            detail: err.to_string(),
                        });
                    }
                }
            }
        }
        Err(err) => {
            checks.push(DoctorCheck {
                name: "auth.manager",
                ok: false,
                detail: err.to_string(),
            });
        }
    }

    if config.enabled {
        let source = if std::env::var("HERMES_SERVER_TOKEN")
            .ok()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
        {
            TokenSource::Environment
        } else {
            TokenSource::None
        };
        checks.push(DoctorCheck {
            name: "auth.token_source",
            ok: true,
            detail: source.to_string(),
        });
    }

    DoctorReport { checks }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hermes_config::ServerConfig;

    #[tokio::test]
    async fn doctor_disabled_server_reports_enabled_check() {
        let config = ServerConfig::default();
        let report = run_doctor(&config, std::env::temp_dir()).await;
        assert!(!report.checks.is_empty());
        assert!(
            report
                .checks
                .iter()
                .any(|c| c.name == "server.enabled" && !c.ok)
        );
    }
}
