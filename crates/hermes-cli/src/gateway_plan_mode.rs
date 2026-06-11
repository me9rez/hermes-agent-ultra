//! Plan mode for messaging gateway sessions (WeCom, Weixin, Telegram, etc.).

use std::sync::Arc;

use hermes_agent::AgentLoop;
use hermes_agent::agent_loop::ToolRegistry as AgentToolRegistry;
use hermes_core::Message;
use hermes_gateway::{Gateway, GatewayError};
use hermes_gateway::gateway::IncomingMessage;
use hermes_tools::PlanPhase;

use hermes_cli::app::bridge_tool_registry;

use crate::gateway_handlers::GatewayHandlerDeps;
use crate::gateway_main::get_or_build_gateway_cached_agent;

/// Parsed `/plan-mode` slash subcommand for IM channels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanModeSlashAction {
    Help,
    On,
    Off,
    Status,
    Approve,
    Reject { feedback: String },
    Edit { plan: String },
    Task { task: String },
}

/// Plain-text approval while a plan is pending (non-slash reply).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanApprovalReply {
    Approve,
    Reject { feedback: Option<String> },
    Edit { plan: String },
}

/// How to run the next gateway agent turn w.r.t. plan mode.
#[derive(Debug, Clone)]
pub enum GatewayPlanTurnPrep {
    /// Run agent with the provided user message.
    Run { user_message: String },
    /// Do not invoke the agent; return this channel reply immediately.
    ReplyOnly { text: String },
}

pub fn parse_plan_mode_slash_args(args: &str) -> PlanModeSlashAction {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return PlanModeSlashAction::Help;
    }
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let sub = parts.next().unwrap_or("").to_ascii_lowercase();
    let rest = parts.next().unwrap_or("").trim().to_string();
    match sub.as_str() {
        "help" | "usage" | "?" => PlanModeSlashAction::Help,
        "on" | "enable" => {
            if rest.is_empty() {
                PlanModeSlashAction::On
            } else {
                PlanModeSlashAction::Task { task: rest }
            }
        }
        "off" | "disable" => PlanModeSlashAction::Off,
        "status" | "show" => PlanModeSlashAction::Status,
        "approve" | "accept" | "a" => PlanModeSlashAction::Approve,
        "reject" | "deny" | "r" => PlanModeSlashAction::Reject { feedback: rest },
        "edit" | "e" => PlanModeSlashAction::Edit { plan: rest },
        _ => PlanModeSlashAction::Task {
            task: trimmed.to_string(),
        },
    }
}

pub fn parse_plan_approval_reply(text: &str) -> Option<PlanApprovalReply> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    match lower.as_str() {
        "approve" | "approved" | "accept" | "a" | "yes" | "y" | "ok" | "好的" | "批准" | "同意"
        | "通过" => Some(PlanApprovalReply::Approve),
        "reject" | "deny" | "r" | "no" | "n" | "拒绝" | "驳回" => {
            Some(PlanApprovalReply::Reject { feedback: None })
        }
        _ if lower.starts_with("reject ") || lower.starts_with("拒绝") => {
            let feedback = trimmed
                .splitn(2, char::is_whitespace)
                .nth(1)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            Some(PlanApprovalReply::Reject { feedback })
        }
        _ if lower.starts_with("edit ") || lower.starts_with("修订") => {
            let plan = trimmed
                .splitn(2, char::is_whitespace)
                .nth(1)
                .unwrap_or("")
                .trim()
                .to_string();
            if plan.is_empty() {
                None
            } else {
                Some(PlanApprovalReply::Edit { plan })
            }
        }
        _ => None,
    }
}

pub fn plan_mode_help_text() -> &'static str {
    "Plan mode (plan-then-execute):\n\
     /plan-mode <task>     Plan with read-only tools, then wait for approval\n\
     /plan-mode on         Enable plan mode for the next task\n\
     /plan-mode off        Disable plan mode\n\
     /plan-mode status     Show current phase\n\
     /plan-mode approve    Approve pending plan and execute\n\
     /plan-mode reject [feedback]  Reject plan\n\
     /plan-mode edit <text>  Revise plan and execute\n\
     While awaiting approval you can also reply: 批准 / approve / 拒绝 / reject"
}

pub fn plan_mode_status_text(agent: &AgentLoop) -> String {
    let phase = agent.plan_phase();
    let pending = agent
        .pending_plan()
        .map(|p| format!("\nPending plan ({} chars).", p.chars().count()))
        .unwrap_or_default();
    format!("Plan mode phase: {}{}", phase.as_str(), pending)
}

pub fn format_plan_pending_channel_reply(plan: &str) -> String {
    format!(
        "📋 Plan submitted — review below, then reply:\n\
         • /plan-mode approve  (or: 批准 / approve)\n\
         • /plan-mode reject [feedback]  (or: 拒绝)\n\
         • /plan-mode edit <revised plan>\n\n\
         ---\n{plan}\n---"
    )
}

pub fn prepare_gateway_plan_turn(agent: &AgentLoop, user_message: &str) -> GatewayPlanTurnPrep {
    if agent.plan_phase() == PlanPhase::AwaitingApproval {
        if let Some(action) = parse_plan_approval_reply(user_message) {
            return match action {
                PlanApprovalReply::Approve => {
                    agent.set_plan_phase(PlanPhase::Executing);
                    GatewayPlanTurnPrep::Run {
                        user_message: "Plan approved. Proceed with execution.".to_string(),
                    }
                }
                PlanApprovalReply::Reject { feedback } => {
                    agent.set_plan_phase(PlanPhase::Planning);
                    agent.set_pending_plan(None);
                    let text = if let Some(fb) = feedback.filter(|s| !s.trim().is_empty()) {
                        format!("Plan rejected. User feedback: {fb}")
                    } else {
                        "Plan rejected. Revise your request.".to_string()
                    };
                    GatewayPlanTurnPrep::Run { user_message: text }
                }
                PlanApprovalReply::Edit { plan } => {
                    agent.set_pending_plan(Some(plan.clone()));
                    agent.set_plan_phase(PlanPhase::Executing);
                    GatewayPlanTurnPrep::Run {
                        user_message: format!("Plan updated and approved:\n{plan}"),
                    }
                }
            };
        }
    }
    GatewayPlanTurnPrep::Run {
        user_message: user_message.to_string(),
    }
}

pub fn finalize_gateway_agent_reply(agent: &AgentLoop, conv: &hermes_agent::ConversationResult) -> String {
    if conv.turn_exit_reason() == "plan_awaiting_approval" {
        if let Some(plan) = agent.pending_plan() {
            return format_plan_pending_channel_reply(&plan);
        }
    }
    conv.final_response
        .as_ref()
        .filter(|s| !s.trim().is_empty())
        .cloned()
        .unwrap_or_default()
}

async fn gateway_agent_for_session(
    gateway: &Gateway,
    incoming: &IncomingMessage,
    session_key: &str,
    deps: &GatewayHandlerDeps,
) -> Result<Arc<tokio::sync::Mutex<AgentLoop>>, GatewayError> {
    let ctx = gateway.runtime_context_for_route(incoming, session_key).await;
    let agent_tools: Arc<AgentToolRegistry> = Arc::new(bridge_tool_registry(&deps.runtime_tools));
    Ok(
        get_or_build_gateway_cached_agent(
            &deps.gateway_agent_cache,
            deps.config.as_ref(),
            &ctx,
            agent_tools,
            deps.runtime_tools.clone(),
        )
        .await,
    )
}

/// Handle `/plan-mode` from a messaging channel (WeCom, Weixin, etc.).
pub async fn execute_plan_mode_slash_command(
    gateway: Arc<Gateway>,
    incoming: &IncomingMessage,
    session_key: &str,
    args: &str,
    deps: GatewayHandlerDeps,
) -> Result<(), GatewayError> {
    let action = parse_plan_mode_slash_args(args);
    let agent_arc = gateway_agent_for_session(&gateway, incoming, session_key, &deps).await?;
    let mut agent = agent_arc.lock().await;

    match action {
        PlanModeSlashAction::Help => {
            gateway
                .send_incoming_reply(incoming, plan_mode_help_text(), None)
                .await?;
        }
        PlanModeSlashAction::On => {
            agent.set_plan_phase(PlanPhase::Planning);
            gateway
                .send_incoming_reply(
                    incoming,
                    "Plan mode ON: agent will research with read-only tools, submit a plan, and wait for approval.",
                    None,
                )
                .await?;
        }
        PlanModeSlashAction::Off => {
            agent.set_plan_phase(PlanPhase::Off);
            agent.set_pending_plan(None);
            gateway
                .send_incoming_reply(incoming, "Plan mode OFF.", None)
                .await?;
        }
        PlanModeSlashAction::Status => {
            let text = plan_mode_status_text(&agent);
            gateway.send_incoming_reply(incoming, &text, None).await?;
        }
        PlanModeSlashAction::Approve => {
            if agent.plan_phase() != PlanPhase::AwaitingApproval {
                gateway
                    .send_incoming_reply(
                        incoming,
                        "No plan awaiting approval. Use /plan-mode <task> first.",
                        None,
                    )
                    .await?;
                return Ok(());
            }
            agent.set_plan_phase(PlanPhase::Executing);
            drop(agent);
            gateway
                .append_user_message_and_route(
                    incoming,
                    session_key,
                    "Plan approved. Proceed with execution.".to_string(),
                )
                .await?;
        }
        PlanModeSlashAction::Reject { feedback } => {
            agent.set_plan_phase(PlanPhase::Planning);
            agent.set_pending_plan(None);
            let user_text = if feedback.trim().is_empty() {
                "Plan rejected. Revise your request.".to_string()
            } else {
                format!("Plan rejected. User feedback: {feedback}")
            };
            drop(agent);
            if feedback.trim().is_empty() {
                gateway.send_incoming_reply(incoming, "Plan rejected.", None).await?;
            } else {
                gateway
                    .append_user_message_and_route(incoming, session_key, user_text)
                    .await?;
            }
        }
        PlanModeSlashAction::Edit { plan } => {
            if plan.trim().is_empty() {
                gateway
                    .send_incoming_reply(
                        incoming,
                        "Usage: /plan-mode edit <revised plan text>",
                        None,
                    )
                    .await?;
                return Ok(());
            }
            agent.set_pending_plan(Some(plan.clone()));
            agent.set_plan_phase(PlanPhase::Executing);
            drop(agent);
            gateway
                .append_user_message_and_route(
                    incoming,
                    session_key,
                    format!("Plan updated and approved:\n{plan}"),
                )
                .await?;
        }
        PlanModeSlashAction::Task { task } => {
            if task.trim().is_empty() {
                gateway
                    .send_incoming_reply(incoming, plan_mode_help_text(), None)
                    .await?;
                return Ok(());
            }
            agent.set_plan_phase(PlanPhase::Planning);
            drop(agent);
            gateway
                .append_user_message_and_route(incoming, session_key, task)
                .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_task_one_liner() {
        assert_eq!(
            parse_plan_mode_slash_args("帮我做推广"),
            PlanModeSlashAction::Task {
                task: "帮我做推广".to_string()
            }
        );
    }

    #[test]
    fn slash_on_with_task() {
        assert_eq!(
            parse_plan_mode_slash_args("on 写测试"),
            PlanModeSlashAction::Task {
                task: "写测试".to_string()
            }
        );
    }

    #[test]
    fn approval_reply_chinese() {
        assert_eq!(
            parse_plan_approval_reply("批准"),
            Some(PlanApprovalReply::Approve)
        );
        assert_eq!(
            parse_plan_approval_reply("拒绝 太复杂"),
            Some(PlanApprovalReply::Reject {
                feedback: Some("太复杂".to_string())
            })
        );
    }
}
