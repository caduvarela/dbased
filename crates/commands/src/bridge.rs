use async_channel::{Receiver, Sender};

use crate::{Command, Event};

/// The UI's ends of the channel pair.
pub struct UiLink {
    pub commands: Sender<Command>,
    pub events: Receiver<Event>,
}

/// The Core's ends of the channel pair.
pub struct CoreLink {
    pub commands: Receiver<Command>,
    pub events: Sender<Event>,
}

/// Creates the bounded channel pair between UI and Core.
pub fn bridge(command_capacity: usize, event_capacity: usize) -> (UiLink, CoreLink) {
    let (command_sender, command_receiver) = async_channel::bounded(command_capacity);
    let (event_sender, event_receiver) = async_channel::bounded(event_capacity);
    let ui_link = UiLink {
        commands: command_sender,
        events: event_receiver,
    };
    let core_link = CoreLink {
        commands: command_receiver,
        events: event_sender,
    };
    (ui_link, core_link)
}
