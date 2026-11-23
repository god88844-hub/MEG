use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
#[derive(Debug, Clone, Default)]
pub struct CancellationToken(pub Arc<AtomicBool>);
impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    pub async fn cancelled(&self) {
        while !self.is_cancelled() {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }
}
