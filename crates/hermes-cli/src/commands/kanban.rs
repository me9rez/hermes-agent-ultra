//! Kanban slash command handler.
//!
//! Provides `render_kanban_status`, `parse_kanban_add`, `handle_kanban_command`,
//! and `run_kanban_command` for the `/kanban` and `/tasks` slash commands.

use std::fmt::Write as _;

use hermes_core::AgentError;

use crate::kanban::{
    KanbanActionInput, KanbanBoard, KanbanLane, NewKanbanTaskInput, add_task, archive_done,
    claim_task, create_or_select_board, ensure_board, find_task_mut, lane_counts, load_store,
    maybe_checkpoint_to_contextlattice, move_task, save_store, set_blocked,
};

use super::background::queue_background_job;
use super::{CommandResult, emit_command_output};

// ---------------------------------------------------------------------------
// Status rendering
// ---------------------------------------------------------------------------

fn render_kanban_status(board: &KanbanBoard) -> String {
    let mut out = String::new();
    let _ = writeln!(
        &mut out,
        "Kanban board: {} ({})",
        board.name.trim(),
        board.id.trim()
    );
    if let Some(project_path) = board.project_path.as_deref() {
        let _ = writeln!(&mut out, "Project: {}", project_path);
    }
    let counts = lane_counts(board);
    let total: usize = counts.iter().map(|(_, count)| *count).sum();
    let _ = writeln!(
        &mut out,
        "Tasks: {} (archived done: {})",
        total,
        board.archived.len()
    );
    for (lane, count) in counts {
        let _ = writeln!(&mut out, "  {:>7}: {}", lane.as_str(), count);
    }
    if board.tasks.is_empty() {
        let _ = writeln!(&mut out, "\nNo active tasks. Use `/kanban add <title>`.");
        return out.trim_end().to_string();
    }

    let mut tasks = board.tasks.clone();
    tasks.sort_by(|a, b| {
        a.lane
            .as_str()
            .cmp(b.lane.as_str())
            .then_with(|| a.priority.cmp(&b.priority))
            .then_with(|| a.id.cmp(&b.id))
    });
    let _ = writeln!(&mut out, "\nActive tasks (top 20):");
    for task in tasks.into_iter().take(20) {
        let assignee = task.assignee.unwrap_or_else(|| "-".to_string());
        let blocked = task
            .blocked_reason
            .as_deref()
            .map(|reason| format!(" blocked={reason}"))
            .unwrap_or_default();
        let bg = task
            .background_job_id
            .as_deref()
            .map(|job_id| format!(" job={job_id}"))
            .unwrap_or_default();
        let _ = writeln!(
            &mut out,
            "- {} [{}] p{} @{} {}{}{}",
            task.id,
            task.lane.as_str(),
            task.priority,
            assignee,
            task.title,
            blocked,
            bg
        );
    }
    out.trim_end().to_string()
}

// ---------------------------------------------------------------------------
// Parse /kanban add arguments
// ---------------------------------------------------------------------------

pub(crate) fn parse_kanban_add(args: &[&str]) -> Result<NewKanbanTaskInput, AgentError> {
    if args.is_empty() {
        return Err(AgentError::Config(
            "Usage: /kanban add <title> [--lane <todo|doing|blocked|done>] [--priority <1..5>] [--assignee <name>] [--depends K-0001,K-0002] [--desc <text>]".to_string(),
        ));
    }
    let mut lane = KanbanLane::Todo;
    let mut priority: u8 = 3;
    let mut assignee: Option<String> = None;
    let mut depends_on: Vec<String> = Vec::new();
    let mut description: Option<String> = None;
    let mut title_parts: Vec<String> = Vec::new();

    let mut idx = 0usize;
    while idx < args.len() {
        let token = args[idx];
        if token == "--lane" {
            idx = idx.saturating_add(1);
            let Some(raw) = args.get(idx) else {
                return Err(AgentError::Config("Missing value for --lane".to_string()));
            };
            lane = KanbanLane::parse(raw).ok_or_else(|| {
                AgentError::Config(format!(
                    "Invalid lane `{raw}`. Use: todo|doing|blocked|done."
                ))
            })?;
        } else if token == "--priority" || token == "-p" {
            idx = idx.saturating_add(1);
            let Some(raw) = args.get(idx) else {
                return Err(AgentError::Config(
                    "Missing value for --priority".to_string(),
                ));
            };
            priority = raw.parse::<u8>().map_err(|_| {
                AgentError::Config(format!("Invalid priority `{raw}`. Expected integer 1..5."))
            })?;
            if !(1..=5).contains(&priority) {
                return Err(AgentError::Config(format!(
                    "Invalid priority `{priority}`. Expected 1..5."
                )));
            }
        } else if token == "--assignee" || token == "-a" {
            idx = idx.saturating_add(1);
            assignee = args.get(idx).map(|s| s.to_string());
        } else if token == "--depends" || token == "--deps" {
            idx = idx.saturating_add(1);
            if let Some(raw) = args.get(idx) {
                depends_on = raw
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string)
                    .collect();
            }
        } else if token == "--desc" || token == "--description" {
            idx = idx.saturating_add(1);
            description = args.get(idx).map(|s| s.to_string());
        } else {
            title_parts.push(token.to_string());
        }
        idx = idx.saturating_add(1);
    }
    let title = title_parts.join(" ").trim().to_string();
    if title.is_empty() {
        return Err(AgentError::Config(
            "Usage: /kanban add <title> [flags...]".to_string(),
        ));
    }
    Ok(NewKanbanTaskInput {
        title,
        lane,
        priority,
        assignee,
        description,
        depends_on,
    })
}

// ---------------------------------------------------------------------------
// Handler (called from handle_slash_command in parent module)
// ---------------------------------------------------------------------------

pub(super) fn handle_kanban_command(
    host: &mut impl crate::app::SlashCommandHost,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    emit_command_output(host, run_kanban_command(args)?);
    Ok(CommandResult::Handled)
}

// ---------------------------------------------------------------------------
// Core public entry-point
// ---------------------------------------------------------------------------

pub fn run_kanban_command(args: &[&str]) -> Result<String, AgentError> {
    let action = args
        .first()
        .copied()
        .unwrap_or("status")
        .to_ascii_lowercase();
    let mut store = load_store()?;

    match action.as_str() {
        "status" | "show" => {
            let requested = args.get(1).copied();
            let board = ensure_board(&mut store, requested);
            return Ok(render_kanban_status(board));
        }
        "boards" | "list" => {
            let mut out = String::from("Kanban boards:\n");
            for board in &store.boards {
                let marker = if board.id == store.current_board_id {
                    "*"
                } else {
                    " "
                };
                let project = board.project_path.as_deref().unwrap_or("-");
                let _ = writeln!(
                    &mut out,
                    "{} {} ({}) project={}",
                    marker, board.name, board.id, project
                );
            }
            return Ok(out.trim_end().to_string());
        }
        "init" => {
            let Some(name) = args.get(1).copied() else {
                return Ok(
                    "Usage: kanban init <board-name> [project-path]\nExample: hermes kanban init alpha ~/Documents/Projects/hermes-agent-ultra"
                        .to_string(),
                );
            };
            let project_path = args.get(2).map(|s| s.to_string());
            let (board_name, board_id, board_snapshot) = {
                let board = create_or_select_board(&mut store, name, project_path);
                (board.name.clone(), board.id.clone(), board.clone())
            };
            save_store(&store)?;
            let checkpoint = maybe_checkpoint_to_contextlattice(
                &board_snapshot,
                KanbanActionInput {
                    action: "init".to_string(),
                    task_id: None,
                    lane: None,
                    summary: format!("board={board_name} board_id={board_id}"),
                },
            );
            return Ok(format!(
                "Board selected: {} ({})\n{}",
                board_name, board_id, checkpoint.detail
            ));
        }
        "use" | "select" => {
            let Some(name_or_id) = args.get(1).copied() else {
                return Ok("Usage: kanban use <board-id-or-name>".to_string());
            };
            let (board_name, board_id) = {
                let board = ensure_board(&mut store, Some(name_or_id));
                (board.name.clone(), board.id.clone())
            };
            save_store(&store)?;
            return Ok(format!("Using board: {} ({})", board_name, board_id));
        }
        "add" => {
            let input = parse_kanban_add(args.get(1..).unwrap_or_default())?;
            let (task_id, task_lane, task_priority, task_title, board_snapshot) = {
                let board = ensure_board(&mut store, None);
                let task = add_task(board, input);
                (
                    task.id.clone(),
                    task.lane,
                    task.priority,
                    task.title.clone(),
                    board.clone(),
                )
            };
            save_store(&store)?;
            let checkpoint = maybe_checkpoint_to_contextlattice(
                &board_snapshot,
                KanbanActionInput {
                    action: "add".to_string(),
                    task_id: Some(task_id.clone()),
                    lane: Some(task_lane),
                    summary: task_title.clone(),
                },
            );
            return Ok(format!(
                "Added task {} [{}] p{}: {}\n{}",
                task_id,
                task_lane.as_str(),
                task_priority,
                task_title,
                checkpoint.detail
            ));
        }
        "move" => {
            let Some(task_ref) = args.get(1).copied() else {
                return Ok(
                    "Usage: kanban move <task-id|title> <todo|doing|blocked|done> [summary]"
                        .to_string(),
                );
            };
            let Some(raw_lane) = args.get(2).copied() else {
                return Ok(
                    "Usage: kanban move <task-id|title> <todo|doing|blocked|done> [summary]"
                        .to_string(),
                );
            };
            let Some(lane) = KanbanLane::parse(raw_lane) else {
                return Ok(format!(
                    "Invalid lane `{raw_lane}`. Use: todo|doing|blocked|done."
                ));
            };
            let summary = args.get(3..).unwrap_or_default().join(" ");
            let maybe_update = {
                let board = ensure_board(&mut store, None);
                let task_meta = if let Some(task) = find_task_mut(board, task_ref) {
                    move_task(
                        task,
                        lane,
                        (!summary.trim().is_empty()).then_some(summary.clone()),
                    );
                    Some((task.id.clone(), task.title.clone()))
                } else {
                    None
                };
                task_meta.map(|(task_id, title)| (task_id, title, board.clone()))
            };
            if let Some((task_id, title, board_snapshot)) = maybe_update {
                save_store(&store)?;
                let checkpoint = maybe_checkpoint_to_contextlattice(
                    &board_snapshot,
                    KanbanActionInput {
                        action: "move".to_string(),
                        task_id: Some(task_id.clone()),
                        lane: Some(lane),
                        summary: format!("{title} {}", summary.trim()).trim().to_string(),
                    },
                );
                return Ok(format!(
                    "Moved {} -> {}\n{}",
                    task_id,
                    lane.as_str(),
                    checkpoint.detail
                ));
            }
            return Ok(format!("Task not found: {task_ref}"));
        }
        "claim" => {
            let Some(task_ref) = args.get(1).copied() else {
                return Ok("Usage: kanban claim <task-id|title> [assignee]".to_string());
            };
            let assignee = args.get(2).map(|s| s.to_string());
            let maybe_update = {
                let board = ensure_board(&mut store, None);
                let task_meta = if let Some(task) = find_task_mut(board, task_ref) {
                    claim_task(task, assignee.clone());
                    Some((task.id.clone(), task.lane))
                } else {
                    None
                };
                task_meta.map(|(task_id, lane)| (task_id, lane, board.clone()))
            };
            if let Some((task_id, lane, board_snapshot)) = maybe_update {
                save_store(&store)?;
                let checkpoint = maybe_checkpoint_to_contextlattice(
                    &board_snapshot,
                    KanbanActionInput {
                        action: "claim".to_string(),
                        task_id: Some(task_id.clone()),
                        lane: Some(lane),
                        summary: format!(
                            "assignee={}",
                            assignee.unwrap_or_else(|| "-".to_string())
                        ),
                    },
                );
                return Ok(format!(
                    "Claimed {} ({})\n{}",
                    task_id, task_ref, checkpoint.detail
                ));
            }
            return Ok(format!("Task not found: {task_ref}"));
        }
        "block" => {
            let Some(task_ref) = args.get(1).copied() else {
                return Ok("Usage: kanban block <task-id|title> <reason>".to_string());
            };
            let reason = args
                .get(2..)
                .unwrap_or_default()
                .join(" ")
                .trim()
                .to_string();
            if reason.is_empty() {
                return Ok("Usage: kanban block <task-id|title> <reason>".to_string());
            }
            let maybe_update = {
                let board = ensure_board(&mut store, None);
                let task_id = if let Some(task) = find_task_mut(board, task_ref) {
                    set_blocked(task, Some(reason.clone()));
                    Some(task.id.clone())
                } else {
                    None
                };
                task_id.map(|task_id| (task_id, board.clone()))
            };
            if let Some((task_id, board_snapshot)) = maybe_update {
                save_store(&store)?;
                let checkpoint = maybe_checkpoint_to_contextlattice(
                    &board_snapshot,
                    KanbanActionInput {
                        action: "block".to_string(),
                        task_id: Some(task_id.clone()),
                        lane: Some(KanbanLane::Blocked),
                        summary: reason,
                    },
                );
                return Ok(format!("Blocked {}\n{}", task_id, checkpoint.detail));
            }
            return Ok(format!("Task not found: {task_ref}"));
        }
        "done" => {
            let Some(task_ref) = args.get(1).copied() else {
                return Ok("Usage: kanban done <task-id|title> [summary]".to_string());
            };
            let summary = args.get(2..).unwrap_or_default().join(" ");
            let maybe_update = {
                let board = ensure_board(&mut store, None);
                let task_id = if let Some(task) = find_task_mut(board, task_ref) {
                    move_task(
                        task,
                        KanbanLane::Done,
                        (!summary.trim().is_empty()).then_some(summary.clone()),
                    );
                    Some(task.id.clone())
                } else {
                    None
                };
                task_id.map(|task_id| (task_id, board.clone()))
            };
            if let Some((task_id, board_snapshot)) = maybe_update {
                save_store(&store)?;
                let checkpoint = maybe_checkpoint_to_contextlattice(
                    &board_snapshot,
                    KanbanActionInput {
                        action: "done".to_string(),
                        task_id: Some(task_id.clone()),
                        lane: Some(KanbanLane::Done),
                        summary,
                    },
                );
                return Ok(format!("Marked done: {}\n{}", task_id, checkpoint.detail));
            }
            return Ok(format!("Task not found: {task_ref}"));
        }
        "archive-done" | "archive" => {
            let (archived, board_snapshot) = {
                let board = ensure_board(&mut store, None);
                let archived = archive_done(board);
                (archived, board.clone())
            };
            save_store(&store)?;
            let checkpoint = maybe_checkpoint_to_contextlattice(
                &board_snapshot,
                KanbanActionInput {
                    action: "archive_done".to_string(),
                    task_id: None,
                    lane: Some(KanbanLane::Done),
                    summary: format!("archived_count={archived}"),
                },
            );
            return Ok(format!(
                "Archived {} done task(s).\n{}",
                archived, checkpoint.detail
            ));
        }
        "dispatch" => {
            let Some(task_ref) = args.get(1).copied() else {
                return Ok(
                    "Usage: kanban dispatch <task-id|title> [background-task-override]".to_string(),
                );
            };
            let override_msg = args.get(2..).unwrap_or_default().join(" ");
            let dispatch_result = {
                let board = ensure_board(&mut store, None);
                if let Some(task) = find_task_mut(board, task_ref) {
                    let task_message = if override_msg.trim().is_empty() {
                        let mut prompt = format!("Execute Kanban task {}: {}", task.id, task.title);
                        if let Some(desc) = task.description.as_deref() {
                            let _ = write!(&mut prompt, "\nDetails: {}", desc.trim());
                        }
                        if !task.depends_on.is_empty() {
                            let _ = write!(
                                &mut prompt,
                                "\nDependencies: {}",
                                task.depends_on.join(", ")
                            );
                        }
                        prompt
                    } else {
                        override_msg.clone()
                    };
                    let job = queue_background_job(&task_message)?;
                    task.background_job_id = Some(job.id.clone());
                    move_task(task, KanbanLane::Doing, None);
                    Some((task.id.clone(), job, task_message, board.clone()))
                } else {
                    None
                }
            };
            if let Some((task_id, job, task_message, board_snapshot)) = dispatch_result {
                save_store(&store)?;
                let checkpoint = maybe_checkpoint_to_contextlattice(
                    &board_snapshot,
                    KanbanActionInput {
                        action: "dispatch".to_string(),
                        task_id: Some(task_id.clone()),
                        lane: Some(KanbanLane::Doing),
                        summary: format!("job_id={} task={}", job.id, task_message),
                    },
                );
                return Ok(format!(
                    "Dispatched {} as background job {}\nStatus: {}\nLogs:   {}\n{}",
                    task_id,
                    job.id,
                    job.status_path.display(),
                    job.log_path.display(),
                    checkpoint.detail
                ));
            }
            return Ok(format!("Task not found: {task_ref}"));
        }
        "sync" => {
            let board_snapshot = {
                let board = ensure_board(&mut store, None);
                board.clone()
            };
            let checkpoint = maybe_checkpoint_to_contextlattice(
                &board_snapshot,
                KanbanActionInput {
                    action: "sync".to_string(),
                    task_id: None,
                    lane: None,
                    summary: format!(
                        "manual sync tasks={} archived={}",
                        board_snapshot.tasks.len(),
                        board_snapshot.archived.len()
                    ),
                },
            );
            return Ok(checkpoint.detail);
        }
        "help" => {
            return Ok(
                "Kanban commands:\n  kanban status [board]\n  kanban boards\n  kanban init <name> [project-path]\n  kanban use <name-or-id>\n  kanban add <title> [--lane <todo|doing|blocked|done>] [--priority <1..5>] [--assignee <name>] [--depends K-0001,K-0002] [--desc <text>]\n  kanban move <task-id|title> <todo|doing|blocked|done> [summary]\n  kanban claim <task-id|title> [assignee]\n  kanban block <task-id|title> <reason>\n  kanban done <task-id|title> [summary]\n  kanban archive-done\n  kanban dispatch <task-id|title> [background-task-override]\n  kanban sync"
                    .to_string(),
            );
        }
        _ => {
            return Ok("Unknown kanban action. Use `hermes kanban help`.".to_string());
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kanban_command_is_registered_and_completable() {
        assert!(
            super::super::SLASH_COMMANDS
                .iter()
                .any(|(name, _)| *name == "/kanban")
        );
        assert!(
            super::super::SLASH_COMMANDS
                .iter()
                .any(|(name, _)| *name == "/tasks")
        );
        let results = super::super::autocomplete("/kan");
        assert!(results.contains(&"/kanban"));
    }

    #[test]
    fn test_parse_kanban_add_defaults() {
        let input = parse_kanban_add(&["Ship", "kanban"]).expect("parse");
        assert_eq!(input.title, "Ship kanban");
        assert_eq!(input.lane, KanbanLane::Todo);
        assert_eq!(input.priority, 3);
    }

    #[test]
    fn test_parse_kanban_add_flags() {
        let input = parse_kanban_add(&[
            "Task",
            "--lane",
            "doing",
            "--priority",
            "2",
            "--assignee",
            "runner",
            "--depends",
            "K-0001,K-0002",
            "--desc",
            "note",
        ])
        .expect("parse");
        assert_eq!(input.title, "Task");
        assert_eq!(input.lane, KanbanLane::Doing);
        assert_eq!(input.priority, 2);
        assert_eq!(input.assignee.as_deref(), Some("runner"));
        assert_eq!(input.depends_on, vec!["K-0001", "K-0002"]);
        assert_eq!(input.description.as_deref(), Some("note"));
    }
}
