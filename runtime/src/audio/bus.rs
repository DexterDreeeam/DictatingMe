use tokio::sync::broadcast;

use super::AudioFrame;

#[derive(Clone)]
pub struct AudioBus {
    sender: broadcast::Sender<AudioFrame>,
}

impl AudioBus {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(8));
        Self { sender }
    }

    pub fn publish(&self, frame: AudioFrame) {
        let _ = self.sender.send(frame);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AudioFrame> {
        self.sender.subscribe()
    }
}
