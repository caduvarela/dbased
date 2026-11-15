//! Test support: one Postgres container per test binary, one database per test.
//! The image comes from the repo-root `.env` so versions never drift (docs/PROJECT.md Part 7).

// @NOTE: each test binary uses a different part of this shared module.
#![allow(dead_code)]
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use dbased_core::drivers::postgres::{PostgresConfig, TlsMode};
use dbased_core::{RowSink, RowSinkMessage, Session, StatementOutcome, UserSql};
use secrecy::SecretString;
use sqlx::{Connection, PgConnection};
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};
use tokio_util::sync::CancellationToken;

pub const PASSWORD: &str = "test-password";

const ENV_FILE: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.env"));

/// `POSTGRES_IMAGE` from `.env`, split into repository and tag.
pub fn postgres_image() -> (String, String) {
    let image = ENV_FILE
        .lines()
        .filter_map(|line| line.trim().strip_prefix("POSTGRES_IMAGE="))
        .next()
        .expect("POSTGRES_IMAGE is declared in .env");
    let (name, tag) = image.split_once(':').unwrap_or((image, "latest"));
    (name.to_owned(), tag.to_owned())
}

/// The running container. It lives on its own thread and runtime so it is
/// removed when the last test using it finishes, whatever runtime that test had.
pub struct Server {
    pub port: u16,
    shutdown: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown.take();
        if let Some(thread) = self.thread.take() {
            thread.join().ok();
        }
    }
}

static SERVER: Mutex<Weak<Server>> = Mutex::new(Weak::new());

fn start_server() -> Arc<Server> {
    let (port_sender, port_receiver) = mpsc::channel();
    let (shutdown_sender, shutdown_receiver) = mpsc::channel::<()>();
    let thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime for the container");
        runtime.block_on(async move {
            let (name, tag) = postgres_image();
            let container = GenericImage::new(name, tag)
                .with_exposed_port(5432.tcp())
                .with_wait_for(WaitFor::message_on_stderr(
                    "database system is ready to accept connections",
                ))
                .with_env_var("POSTGRES_PASSWORD", PASSWORD)
                .start()
                .await
                .expect("container starts");
            let port = container
                .get_host_port_ipv4(5432.tcp())
                .await
                .expect("mapped port");
            port_sender.send(port).ok();
            // Blocks until every test dropped its handle (the sender is dropped).
            shutdown_receiver.recv().ok();
            container.rm().await.ok();
        });
    });
    let port = port_receiver.recv().expect("container port");
    Arc::new(Server {
        port,
        shutdown: Some(shutdown_sender),
        thread: Some(thread),
    })
}

fn acquire_server() -> Arc<Server> {
    let mut slot = SERVER.lock().expect("server lock");
    if let Some(server) = slot.upgrade() {
        return server;
    }
    let server = start_server();
    *slot = Arc::downgrade(&server);
    server
}

pub struct TestDatabase {
    pub server: Arc<Server>,
    pub name: String,
}

static DATABASE_COUNTER: AtomicU32 = AtomicU32::new(0);

impl TestDatabase {
    pub async fn create() -> Self {
        let server = acquire_server();
        let name = format!(
            "test_{}_{}",
            std::process::id(),
            DATABASE_COUNTER.fetch_add(1, Ordering::SeqCst)
        );
        let mut admin = connect_admin(&server).await;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE DATABASE \"{name}\"")))
            .execute(&mut admin)
            .await
            .expect("create database");
        admin.close().await.ok();
        Self { server, name }
    }

    /// Plaintext on purpose: the test container has no TLS.
    pub fn config(&self) -> PostgresConfig {
        let mut config = PostgresConfig::new(
            "127.0.0.1",
            self.server.port,
            "postgres",
            SecretString::from(PASSWORD),
            self.name.clone(),
        );
        config.tls = TlsMode::Disable;
        config
    }

    pub async fn admin(&self) -> PgConnection {
        connect_admin(&self.server).await
    }
}

async fn connect_admin(server: &Server) -> PgConnection {
    let options = sqlx::postgres::PgConnectOptions::new_without_pgpass()
        .host("127.0.0.1")
        .port(server.port)
        .username("postgres")
        .password(PASSWORD)
        .database("postgres")
        .ssl_mode(sqlx::postgres::PgSslMode::Disable);
    // The readiness log line also appears for the init-time server, so retry.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match PgConnection::connect_with(&options).await {
            Ok(connection) => return connection,
            Err(error) if Instant::now() > deadline => {
                panic!("postgres never became ready: {error}")
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
        }
    }
}

#[derive(Debug)]
pub struct Collected {
    pub columns: Vec<dbased_commands::ColumnMeta>,
    pub rows: Vec<Vec<dbased_commands::Value>>,
    pub outcome: StatementOutcome,
}

/// Runs one statement, consuming the sink concurrently like the Core's buffer would.
pub async fn run_with(
    session: &mut Box<dyn Session>,
    sql: &str,
    token: CancellationToken,
) -> Collected {
    let (sink, mut receiver) = RowSink::channel(8);
    let collecting = async {
        let mut columns = Vec::new();
        let mut rows = Vec::new();
        let mut outcome = None;
        while let Some(message) = receiver.recv().await {
            match message {
                RowSinkMessage::Columns(found) => columns = found,
                RowSinkMessage::Row(row) => rows.push(row),
                RowSinkMessage::Outcome(found) => outcome = Some(found),
            }
        }
        (columns, rows, outcome)
    };
    let executing = session.execute(UserSql::new(sql), sink, token);
    let (result, (columns, rows, streamed_outcome)) = tokio::join!(executing, collecting);
    let outcome = result.expect("execute returns an outcome");
    assert_eq!(
        streamed_outcome.as_ref(),
        Some(&outcome),
        "sink and return agree"
    );
    Collected {
        columns,
        rows,
        outcome,
    }
}

pub async fn run(session: &mut Box<dyn Session>, sql: &str) -> Collected {
    run_with(session, sql, CancellationToken::new()).await
}
