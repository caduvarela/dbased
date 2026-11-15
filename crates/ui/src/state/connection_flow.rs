use std::collections::HashMap;

use dbased_commands::{ConnectionId, ErrorInfo, ErrorKind, ExecuteTarget, RequestId, TabId};

/// The dialog's "Test connection" result, straight from the request's terminal event.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TestStatus {
    #[default]
    Idle,
    Testing,
    Success,
    Failed(String),
}

/// Where a `SaveConnection` from the connection dialog stands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SaveStatus {
    #[default]
    Idle,
    Saving,
    Saved(ConnectionId),
    Failed(String),
}

/// An `Execute` the Core refused for want of a password: replayed once connected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingExecute {
    pub tab: TabId,
    pub script: String,
    pub target: ExecuteTarget,
}

/// The password prompt (docs/PROJECT.md 5.1): shown when a Connect or an Execute fails
/// with `PasswordRequired`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordPrompt {
    pub connection: ConnectionId,
    pub resume: Option<PendingExecute>,
}

/// What a delete confirmation tells the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteConfirmation {
    pub connection: ConnectionId,
    pub name: String,
    pub open_tabs: usize,
}

impl DeleteConfirmation {
    pub fn message(&self) -> String {
        let base = format!("Delete the connection \"{}\"?", self.name);
        match self.open_tabs {
            0 => base,
            1 => format!("{base} Its 1 open tab will be closed."),
            count => format!("{base} Its {count} open tabs will be closed."),
        }
    }
}

/// What a request in flight was for, so its terminal event can be routed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pending {
    Test,
    Save,
    /// A save the user did not see: "remember this password" from the prompt.
    SilentSave,
    Delete,
    Disconnect,
    Connect {
        connection: ConnectionId,
        resume: Option<PendingExecute>,
    },
}

/// What the state must do after a request ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Not a connection request, or nothing more to do.
    None,
    Replay(PendingExecute),
    Notice(String),
}

/// Requests of the connection features and their dialogs' progress (docs/PROJECT.md
/// 11.1), free of GPUI.
#[derive(Debug, Default)]
pub struct ConnectionFlow {
    pub test: TestStatus,
    pub save: SaveStatus,
    pub password_prompt: Option<PasswordPrompt>,
    pending: HashMap<RequestId, Pending>,
    executes: HashMap<RequestId, PendingExecute>,
}

impl ConnectionFlow {
    /// A fresh dialog starts without the last one's results.
    pub fn reset_dialog(&mut self) {
        self.test = TestStatus::Idle;
        self.save = SaveStatus::Idle;
    }

    pub fn track(&mut self, request: RequestId, pending: Pending) {
        self.pending.insert(request, pending);
    }

    pub fn track_execute(&mut self, request: RequestId, execute: PendingExecute) {
        self.executes.insert(request, execute);
    }

    /// Forgets an execution that ended; `Some` when it was tracked.
    pub fn take_execute(&mut self, request: RequestId) -> Option<PendingExecute> {
        self.executes.remove(&request)
    }

    pub fn saved(&mut self, request: RequestId, id: ConnectionId) {
        if let Some(Pending::Save) = self.pending.remove(&request) {
            self.save = SaveStatus::Saved(id);
        }
    }

    pub fn completed(&mut self, request: RequestId) -> Outcome {
        match self.pending.remove(&request) {
            Some(Pending::Test) => {
                self.test = TestStatus::Success;
                Outcome::None
            }
            Some(Pending::Connect {
                resume: Some(execute),
                ..
            }) => Outcome::Replay(execute),
            _ => Outcome::None,
        }
    }

    /// `None` when the request is not one of ours.
    pub fn failed(&mut self, request: RequestId, error: &ErrorInfo) -> Option<Outcome> {
        let pending = self.pending.remove(&request)?;
        let outcome = match pending {
            Pending::Test => {
                self.test = TestStatus::Failed(error.message.clone());
                Outcome::None
            }
            Pending::Save => {
                self.save = SaveStatus::Failed(error.message.clone());
                Outcome::None
            }
            Pending::Connect { connection, resume } => {
                if error.kind == ErrorKind::PasswordRequired {
                    self.ask_password(connection, resume);
                    Outcome::None
                } else {
                    Outcome::Notice(error.message.clone())
                }
            }
            Pending::SilentSave | Pending::Delete | Pending::Disconnect => {
                Outcome::Notice(error.message.clone())
            }
        };
        Some(outcome)
    }

    pub fn ask_password(&mut self, connection: ConnectionId, resume: Option<PendingExecute>) {
        self.password_prompt = Some(PasswordPrompt { connection, resume });
    }
}
