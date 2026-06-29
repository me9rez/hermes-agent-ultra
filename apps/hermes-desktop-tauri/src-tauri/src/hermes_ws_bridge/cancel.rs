use super::conn::HermesWsBridge;
use super::types::{ClientFrame, ClientFrameKind, StreamId};

impl HermesWsBridge {
    pub async fn cancel_stream(&self, stream_id: StreamId) -> Result<(), String> {
        let frame = ClientFrame {
            stream_id: stream_id.clone(),
            kind: ClientFrameKind::Abort,
            payload: None,
        };
        self.send_client_frame(frame).await?;
        self.router.unsubscribe(&stream_id);
        Ok(())
    }
}
