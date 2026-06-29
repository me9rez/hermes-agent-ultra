use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::broadcast;

use super::types::{ServerFrame, StreamId};

pub struct StreamRouter {
    channels: DashMap<StreamId, broadcast::Sender<ServerFrame>>,
}

impl Default for StreamRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamRouter {
    pub fn new() -> Self {
        Self {
            channels: DashMap::new(),
        }
    }

    pub fn subscribe(&self, stream_id: StreamId) -> broadcast::Receiver<ServerFrame> {
        if let Some(tx) = self.channels.get(&stream_id) {
            return tx.subscribe();
        }
        let (tx, rx) = broadcast::channel(256);
        self.channels.insert(stream_id, tx);
        rx
    }

    pub fn unsubscribe(&self, stream_id: &StreamId) {
        self.channels.remove(stream_id);
    }

    pub fn publish(&self, stream_id: &StreamId, frame: ServerFrame) {
        if let Some(tx) = self.channels.get(stream_id) {
            let _ = tx.send(frame);
        }
    }

    pub fn active_streams(&self) -> Vec<StreamId> {
        self.channels.iter().map(|e| e.key().clone()).collect()
    }

    pub fn shared() -> Arc<Self> {
        Arc::new(Self::new())
    }
}
