use std::path::{Path, PathBuf};

use dbased_commands::{ConnectionId, ConnectionSettings, DatabaseKind, TlsMode};
use etcetera::BaseStrategy;
use serde::{Deserialize, Serialize};

use crate::CoreError;

const CONNECTIONS_KEY: &str = "connections";

/// One `[[connections]]` entry of the config file (docs/PROJECT.md 5.2). Never holds a password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredConnection {
    pub id: ConnectionId,
    pub settings: ConnectionSettings,
    pub password_saved: bool,
}

#[derive(Serialize, Deserialize)]
struct ConnectionRecord {
    id: u64,
    name: String,
    kind: KindRecord,
    host: String,
    port: u16,
    user: String,
    database: String,
    tls: TlsRecord,
    #[serde(default)]
    password_saved: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum KindRecord {
    Postgres,
    Mysql,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum TlsRecord {
    Disable,
    Prefer,
    Require,
}

impl From<&StoredConnection> for ConnectionRecord {
    fn from(stored: &StoredConnection) -> Self {
        let settings = &stored.settings;
        Self {
            id: stored.id.0,
            name: settings.name.clone(),
            kind: match settings.kind {
                DatabaseKind::Postgres => KindRecord::Postgres,
                DatabaseKind::MySql => KindRecord::Mysql,
            },
            host: settings.host.clone(),
            port: settings.port,
            user: settings.user.clone(),
            database: settings.database.clone(),
            tls: match settings.tls {
                TlsMode::Disable => TlsRecord::Disable,
                TlsMode::Prefer => TlsRecord::Prefer,
                TlsMode::Require => TlsRecord::Require,
            },
            password_saved: stored.password_saved,
        }
    }
}

impl From<ConnectionRecord> for StoredConnection {
    fn from(record: ConnectionRecord) -> Self {
        Self {
            id: ConnectionId(record.id),
            settings: ConnectionSettings {
                name: record.name,
                kind: match record.kind {
                    KindRecord::Postgres => DatabaseKind::Postgres,
                    KindRecord::Mysql => DatabaseKind::MySql,
                },
                host: record.host,
                port: record.port,
                user: record.user,
                database: record.database,
                tls: match record.tls {
                    TlsRecord::Disable => TlsMode::Disable,
                    TlsRecord::Prefer => TlsMode::Prefer,
                    TlsRecord::Require => TlsMode::Require,
                },
            },
            password_saved: record.password_saved,
        }
    }
}

/// The connection list inside the TOML config file. The path is injected; other tables of the
/// file (settings, workspace) are preserved on every write.
#[derive(Debug, Clone)]
pub struct ConnectionStore {
    path: PathBuf,
}

impl ConnectionStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// `<config dir>/dbased/config.toml` (XDG on Linux).
    pub fn default_path() -> Result<PathBuf, CoreError> {
        let strategy = etcetera::choose_base_strategy()
            .map_err(|error| CoreError::Storage(format!("no config directory: {error}")))?;
        Ok(strategy.config_dir().join("dbased").join("config.toml"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A missing file is an empty list; an unreadable or malformed one is an error.
    pub fn load(&self) -> Result<Vec<StoredConnection>, CoreError> {
        let mut document = self.read_document()?;
        let Some(value) = document.remove(CONNECTIONS_KEY) else {
            return Ok(Vec::new());
        };
        let records: Vec<ConnectionRecord> = value
            .try_into()
            .map_err(|error| CoreError::Storage(format!("invalid [[connections]]: {error}")))?;
        Ok(records.into_iter().map(StoredConnection::from).collect())
    }

    /// Atomic replace: write a temp file next to the target, then rename it over.
    pub fn save(&self, connections: &[StoredConnection]) -> Result<(), CoreError> {
        let mut document = self.read_document()?;
        let records: Vec<ConnectionRecord> =
            connections.iter().map(ConnectionRecord::from).collect();
        let value = toml::Value::try_from(records)
            .map_err(|error| CoreError::Storage(format!("cannot encode connections: {error}")))?;
        document.insert(CONNECTIONS_KEY.to_owned(), value);
        let text = toml::to_string_pretty(&document)
            .map_err(|error| CoreError::Storage(format!("cannot encode config: {error}")))?;
        self.write_atomically(&text)
    }

    fn read_document(&self) -> Result<toml::Table, CoreError> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(toml::Table::new());
            }
            Err(error) => return Err(storage_error("cannot read config", &error)),
        };
        text.parse::<toml::Table>()
            .map_err(|error| CoreError::Storage(format!("invalid config file: {error}")))
    }

    fn write_atomically(&self, text: &str) -> Result<(), CoreError> {
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)
            .map_err(|error| storage_error("cannot create config directory", &error))?;
        let mut temporary = self.path.clone().into_os_string();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        std::fs::write(&temporary, text)
            .map_err(|error| storage_error("cannot write config", &error))?;
        std::fs::rename(&temporary, &self.path).map_err(|error| {
            std::fs::remove_file(&temporary).ok();
            storage_error("cannot replace config", &error)
        })
    }
}

fn storage_error(context: &str, error: &std::io::Error) -> CoreError {
    CoreError::Storage(format!("{context}: {error}"))
}
