use async_channel::{Sender, TrySendError};
use dbased_commands::Command;

use crate::state::SendFailure;

/// The UI's way to talk to the Core: never blocks, reports a full or closed channel.
#[derive(Clone)]
pub struct CommandSink {
    sender: Sender<Command>,
}

impl CommandSink {
    pub fn new(sender: Sender<Command>) -> Self {
        Self { sender }
    }

    pub fn send(&self, command: Command) -> Result<(), SendFailure> {
        self.sender.try_send(command).map_err(|error| match error {
            TrySendError::Full(_) => SendFailure::Full,
            TrySendError::Closed(_) => SendFailure::Closed,
        })
    }
}
