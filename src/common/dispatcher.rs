use std::fmt::Debug;
use thiserror::Error;
use tokio::sync::broadcast::error::{RecvError, SendError};

/// Errors that can occur during a [`Dispatcher`]'s workflow.
#[derive(Debug, Error)]
pub enum DispatcherError<T: Debug> {
    /// An error occurred while receiving a message.
    #[error("Receive error: {:?}", self)]
    Receiving(RecvError),
    /// An error occurred while sending a message.
    #[error("Send error: {:?}", self)]
    Dispatching(SendError<T>),
}

/// Defines shared functionality for a dispatcher.
pub trait Dispatcher<T: Debug> {
    /// The output type of the dispatcher.
    type Output: Clone + Send + Sync;

    /// Dispatches a message.
    fn dispatch(&self, message: &Self::Output) -> Result<usize, DispatcherError<T>>;
}
