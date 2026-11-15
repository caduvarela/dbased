use async_channel::{Sender, TrySendError};
use dbased_commands::Command;
use gpui::{App, Entity};

use crate::state::{SendFailure, UiState};

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

/// Sends commands; a full or closed channel becomes a notice, never a freeze.
pub fn dispatch(sink: &CommandSink, state: &Entity<UiState>, commands: Vec<Command>, cx: &mut App) {
    for command in commands {
        if let Err(failure) = sink.send(command) {
            state.update(cx, |state, cx| {
                state.send_failed(failure);
                cx.notify();
            });
            return;
        }
    }
}
