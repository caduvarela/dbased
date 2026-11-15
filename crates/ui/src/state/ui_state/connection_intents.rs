use dbased_commands::{Command, ConnectionId, PasswordSource, PasswordUpdate};
use secrecy::SecretString;

use super::UiState;
use crate::state::{
    connection_flow::{DeleteConfirmation, Outcome, Pending, SaveStatus, TestStatus},
    connection_form::{ConnectionForm, FormErrors},
};

/// User intents about connections (docs/PROJECT.md 11.1).
impl UiState {
    /// Starts a connection test. Nothing is sent while the form is invalid or a test is
    /// already running.
    pub fn test_connection(&mut self, form: &ConnectionForm) -> Result<Vec<Command>, FormErrors> {
        let settings = form.settings()?;
        if self.flow.test == TestStatus::Testing {
            return Ok(Vec::new());
        }
        let request = self.ids.request();
        self.flow.test = TestStatus::Testing;
        self.flow.track(request, Pending::Test);
        Ok(vec![Command::TestConnection {
            request,
            id: form.editing(),
            settings,
            password: form.password_source(),
        }])
    }

    pub fn save_connection(&mut self, form: &ConnectionForm) -> Result<Vec<Command>, FormErrors> {
        let settings = form.settings()?;
        if self.flow.save == SaveStatus::Saving {
            return Ok(Vec::new());
        }
        let request = self.ids.request();
        self.flow.save = SaveStatus::Saving;
        self.flow.track(request, Pending::Save);
        Ok(vec![Command::SaveConnection {
            request,
            id: form.editing(),
            settings,
            password: form.password_update(),
        }])
    }

    /// Always asks for the saved password: the Core answers `PasswordRequired` when
    /// there is none, and the prompt takes over.
    pub fn connect(&mut self, connection: ConnectionId) -> Vec<Command> {
        if self.connections.get(connection).is_none() {
            return Vec::new();
        }
        let request = self.ids.request();
        self.flow.track(
            request,
            Pending::Connect {
                connection,
                resume: None,
            },
        );
        vec![Command::Connect {
            request,
            id: connection,
            password: PasswordSource::Saved,
        }]
    }

    pub fn disconnect(&mut self, connection: ConnectionId) -> Vec<Command> {
        if self.connections.get(connection).is_none() {
            return Vec::new();
        }
        let request = self.ids.request();
        self.flow.track(request, Pending::Disconnect);
        vec![Command::Disconnect {
            request,
            id: connection,
        }]
    }

    pub fn delete_confirmation(&self, connection: ConnectionId) -> Option<DeleteConfirmation> {
        self.connections.get(connection)?;
        let open_tabs = self
            .tabs
            .iter()
            .filter(|tab| tab.connection == connection)
            .count();
        Some(DeleteConfirmation {
            connection,
            name: self.connections.name(connection),
            open_tabs,
        })
    }

    /// Closes the connection's tabs first, then deletes it (docs/PROJECT.md 9.1).
    pub fn delete_connection(&mut self, connection: ConnectionId) -> Vec<Command> {
        if self.connections.get(connection).is_none() {
            return Vec::new();
        }
        let tabs: Vec<_> = self
            .tabs
            .iter()
            .filter(|tab| tab.connection == connection)
            .map(|tab| tab.id)
            .collect();
        let mut commands: Vec<Command> = tabs
            .into_iter()
            .flat_map(|tab| self.close_tab(tab))
            .collect();
        let request = self.ids.request();
        self.flow.track(request, Pending::Delete);
        commands.push(Command::DeleteConnection {
            request,
            id: connection,
        });
        commands
    }

    /// Connects with the typed password for this session; `save` also stores it.
    pub fn submit_password(&mut self, password: SecretString, save: bool) -> Vec<Command> {
        let Some(prompt) = self.flow.password_prompt.take() else {
            return Vec::new();
        };
        let connection = prompt.connection;
        let Some(settings) = self
            .connections
            .get(connection)
            .map(|summary| summary.settings.clone())
        else {
            return Vec::new();
        };
        let request = self.ids.request();
        self.flow.track(
            request,
            Pending::Connect {
                connection,
                resume: prompt.resume,
            },
        );
        let mut commands = vec![Command::Connect {
            request,
            id: connection,
            password: PasswordSource::Typed(password.clone()),
        }];
        if save {
            let request = self.ids.request();
            self.flow.track(request, Pending::SilentSave);
            commands.push(Command::SaveConnection {
                request,
                id: Some(connection),
                settings,
                password: PasswordUpdate::Save(password),
            });
        }
        commands
    }

    pub fn cancel_password(&mut self) {
        self.flow.password_prompt = None;
    }

    pub(super) fn connection_request_completed(
        &mut self,
        request: dbased_commands::RequestId,
    ) -> Vec<Command> {
        match self.flow.completed(request) {
            Outcome::Replay(execute) => self.execute(execute.tab, execute.script, execute.target),
            Outcome::Notice(message) => {
                self.notice = Some(message);
                Vec::new()
            }
            Outcome::None => Vec::new(),
        }
    }
}
