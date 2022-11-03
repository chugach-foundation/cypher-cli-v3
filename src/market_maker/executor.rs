use std::sync::Arc;

use tokio::sync::broadcast::Sender;

use super::{context::ContextBuilder, hedging::Hedger, making::Maker};

/// Represents an executor with two different operations.
///
/// One of these operations is `makeable` and the other is `hedgeable`.
pub struct Executor<M, H> {
    maker: Arc<M>,
    hedger: Arc<H>,
    context_builder: Arc<ContextBuilder>,
    shutdown: Arc<Sender<bool>>,
}

impl<M, H> Executor<M, H>
where
    M: Maker + Send + Sync,
    H: Hedger + Send + Sync,
{
    /// Creates a new [`Executor`].
    pub fn new(
        maker: Arc<M>,
        hedger: Arc<H>,
        context_builder: Arc<ContextBuilder>,
        shutdown: Arc<Sender<bool>>,
    ) -> Self {
        Self {
            maker,
            hedger,
            context_builder,
            shutdown,
        }
    }
}
