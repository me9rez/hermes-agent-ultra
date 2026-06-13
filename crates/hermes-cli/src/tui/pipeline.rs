use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossterm::event::Event as CrosstermEvent;
use hermes_core::AgentError;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::event::Event;
use crate::app::actors::AgentLane;
pub(crate) async fn abort_and_join_task(task: &mut Option<JoinHandle<()>>) {
    if let Some(handle) = task.take() {
        handle.abort();
        let _ = handle.await;
    }
}

pub(crate) async fn abort_agent_lanes(
    agent_lane: &AgentLane,
    managed_task: &mut Option<JoinHandle<()>>,
) {
    agent_lane.abort();
    abort_and_join_task(managed_task).await;
}

/// Blocking crossterm reader on a dedicated OS thread, bridged into the async event loop.
pub(crate) struct CrosstermEventPipeline {
    shutdown: Arc<AtomicBool>,
    blocking_tx: mpsc::UnboundedSender<Event>,
    reader_join: Option<std::thread::JoinHandle<()>>,
    bridge_task: JoinHandle<()>,
}

pub(crate) fn spawn_crossterm_event_pipeline(
    event_sender: mpsc::UnboundedSender<Event>,
) -> Result<CrosstermEventPipeline, AgentError> {
    let shutdown = Arc::new(AtomicBool::new(false));
    let (blocking_tx, mut blocking_rx) = mpsc::unbounded_channel();
    let shutdown_reader = shutdown.clone();
    let blocking_tx_reader = blocking_tx.clone();

    let reader_join = std::thread::Builder::new()
        .name("hermes-crossterm-reader".into())
        .spawn(move || {
            while !shutdown_reader.load(Ordering::Relaxed) {
                if crate::checklist::embedded_picker_active() {
                    std::thread::sleep(Duration::from_millis(16));
                    continue;
                }
                if crossterm::event::poll(Duration::from_millis(16)).unwrap_or(false) {
                    if let Ok(event) = crossterm::event::read() {
                        let msg = match event {
                            CrosstermEvent::Key(key) if crate::key_event_is_actionable(&key) => {
                                Some(Event::Key(key))
                            }
                            CrosstermEvent::Resize(w, h) => Some(Event::Resize(w, h)),
                            CrosstermEvent::Mouse(mouse) => Some(Event::Mouse(mouse)),
                            CrosstermEvent::Paste(text) => Some(Event::Paste(text)),
                            _ => None,
                        };
                        if let Some(msg) = msg {
                            if blocking_tx_reader.send(msg).is_err() {
                                return;
                            }
                        }
                    }
                }
            }
        })
        .map_err(|e| AgentError::Config(format!("failed to spawn crossterm reader: {e}")))?;

    let bridge_task = tokio::spawn(async move {
        while let Some(msg) = blocking_rx.recv().await {
            if event_sender.send(msg).is_err() {
                break;
            }
        }
    });

    Ok(CrosstermEventPipeline {
        shutdown,
        blocking_tx,
        reader_join: Some(reader_join),
        bridge_task,
    })
}

pub(crate) async fn shutdown_crossterm_event_pipeline(mut pipeline: CrosstermEventPipeline) {
    pipeline.shutdown.store(true, Ordering::Release);
    drop(pipeline.blocking_tx);
    if let Some(reader) = pipeline.reader_join.take() {
        if let Err(err) = reader.join() {
            tracing::warn!("crossterm reader thread panicked: {err:?}");
        }
    }
    let _ = pipeline.bridge_task.await;
}

pub(crate) struct SignalBridge {
    task: JoinHandle<()>,
    cancel_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

pub(crate) fn spawn_signal_bridge(signal_sender: mpsc::UnboundedSender<Event>) -> SignalBridge {
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut cancel_rx = cancel_rx;
        tokio::select! {
            _ = &mut cancel_rx => {}
            _ = async {
                #[cfg(unix)]
                {
                    use tokio::signal::unix::{signal, SignalKind};
                    let mut sigint = signal(SignalKind::interrupt()).ok();
                    let mut sigterm = signal(SignalKind::terminate()).ok();
                    let mut sighup = signal(SignalKind::hangup()).ok();
                    tokio::select! {
                        _ = async {
                            if let Some(sig) = sigint.as_mut() {
                                let _ = sig.recv().await;
                            } else {
                                std::future::pending::<()>().await;
                            }
                        } => {}
                        _ = async {
                            if let Some(sig) = sigterm.as_mut() {
                                let _ = sig.recv().await;
                            } else {
                                std::future::pending::<()>().await;
                            }
                        } => {}
                        _ = async {
                            if let Some(sig) = sighup.as_mut() {
                                let _ = sig.recv().await;
                            } else {
                                std::future::pending::<()>().await;
                            }
                        } => {}
                    }
                }
                #[cfg(not(unix))]
                {
                    let _ = tokio::signal::ctrl_c().await;
                }
                let _ = signal_sender.send(Event::Shutdown);
            } => {}
        }
    });
    SignalBridge {
        task,
        cancel_tx: Some(cancel_tx),
    }
}

pub(crate) async fn shutdown_signal_bridge(bridge: SignalBridge) {
    if let Some(tx) = bridge.cancel_tx {
        let _ = tx.send(());
    }
    let _ = bridge.task.await;
}
