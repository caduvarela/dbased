//! MySQL containers: one server with TLS and one without, each shared by the tests of a binary.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use dbased_core::drivers::mysql::{MysqlConfig, TlsMode};
use secrecy::SecretString;
use sqlx::mysql::{MySqlConnectOptions, MySqlSslMode};
use sqlx::{Connection, MySqlConnection};
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};

pub const PASSWORD: &str = "test-password";

const ENV_FILE: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.env"));

/// `MYSQL_IMAGE` from `.env`, split into repository and tag.
pub fn mysql_image() -> (String, String) {
    let image = ENV_FILE
        .lines()
        .filter_map(|line| line.trim().strip_prefix("MYSQL_IMAGE="))
        .next()
        .expect("MYSQL_IMAGE is declared in .env");
    let (name, tag) = image.split_once(':').unwrap_or((image, "latest"));
    (name.to_owned(), tag.to_owned())
}

/// A running container. It lives on its own thread and runtime so it is removed when the
/// last test using it finishes, whatever runtime that test had.
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

static TLS_SERVER: Mutex<Weak<Server>> = Mutex::new(Weak::new());
static PLAIN_SERVER: Mutex<Weak<Server>> = Mutex::new(Weak::new());

fn start_server(arguments: &'static [&'static str]) -> Arc<Server> {
    let (port_sender, port_receiver) = mpsc::channel();
    let (shutdown_sender, shutdown_receiver) = mpsc::channel::<()>();
    let thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime for the container");
        runtime.block_on(async move {
            let (name, tag) = mysql_image();
            let container = GenericImage::new(name, tag)
                .with_exposed_port(3306.tcp())
                // The init-time server logs the same line with port 0, so wait for the real one.
                .with_wait_for(WaitFor::message_on_stderr("port: 3306  MySQL Community"))
                .with_env_var("MYSQL_ROOT_PASSWORD", PASSWORD)
                .with_cmd(arguments.iter().copied())
                .start()
                .await
                .expect("container starts");
            let port = container
                .get_host_port_ipv4(3306.tcp())
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

fn acquire(slot: &Mutex<Weak<Server>>, arguments: &'static [&'static str]) -> Arc<Server> {
    let mut slot = slot.lock().expect("server lock");
    if let Some(server) = slot.upgrade() {
        return server;
    }
    let server = start_server(arguments);
    *slot = Arc::downgrade(&server);
    server
}

pub struct TestDatabase {
    pub server: Arc<Server>,
    pub name: String,
    /// Whether the server offers TLS.
    pub has_tls: bool,
}

static DATABASE_COUNTER: AtomicU32 = AtomicU32::new(0);

impl TestDatabase {
    /// A database on a server that offers TLS (the image default, a self-signed certificate).
    pub async fn create() -> Self {
        Self::create_on(acquire(&TLS_SERVER, &[]), true).await
    }

    /// A database on a server whose TLS is switched off (an empty `tls_version`).
    // @NOTE: without TLS sqlx cannot do the RSA exchange `caching_sha2_password` needs, so this
    // server authenticates root with `mysql_native_password`.
    pub async fn create_without_tls() -> Self {
        Self::create_on(
            acquire(
                &PLAIN_SERVER,
                &[
                    "--tls-version=",
                    "--mysql-native-password=ON",
                    "--authentication-policy=mysql_native_password",
                ],
            ),
            false,
        )
        .await
    }

    async fn create_on(server: Arc<Server>, has_tls: bool) -> Self {
        let name = format!(
            "test_{}_{}",
            std::process::id(),
            DATABASE_COUNTER.fetch_add(1, Ordering::SeqCst)
        );
        let mut admin = connect_admin(&server, has_tls, None).await;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE DATABASE `{name}`")))
            .execute(&mut admin)
            .await
            .expect("create database");
        admin.close().await.ok();
        Self {
            server,
            name,
            has_tls,
        }
    }

    /// TLS is required whenever the server offers it, so the default path is the one tested.
    pub fn config(&self) -> MysqlConfig {
        let mut config = MysqlConfig::new(
            "127.0.0.1",
            self.server.port,
            "root",
            SecretString::from(PASSWORD),
            self.name.clone(),
        );
        config.tls = if self.has_tls {
            TlsMode::Require
        } else {
            TlsMode::Disable
        };
        config
    }

    /// A plain connection with no default database.
    pub async fn admin(&self) -> MySqlConnection {
        connect_admin(&self.server, self.has_tls, None).await
    }

    /// Runs setup statements inside this test's database.
    pub async fn setup(&self, statements: &[&str]) {
        let mut admin = connect_admin(&self.server, self.has_tls, Some(&self.name)).await;
        for statement in statements {
            sqlx::raw_sql(sqlx::AssertSqlSafe((*statement).to_owned()))
                .execute(&mut admin)
                .await
                .expect("setup statement");
        }
        admin.close().await.ok();
    }

    /// A table `nums(n)` holding 1..=`count`, for large cross-join results.
    pub async fn create_numbers(&self, count: u32) {
        let fill = format!(
            "INSERT INTO nums WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL \
             SELECT n + 1 FROM s WHERE n < {count}) SELECT n FROM s"
        );
        self.setup(&[
            "CREATE TABLE nums (n INT NOT NULL PRIMARY KEY)",
            "SET SESSION cte_max_recursion_depth = 100000",
            &fill,
        ])
        .await;
    }

    /// Counts server connections with this id.
    pub async fn connection_count(&self, id: u64) -> i64 {
        let mut admin = self.admin().await;
        sqlx::query_scalar("SELECT COUNT(*) FROM information_schema.PROCESSLIST WHERE ID = ?")
            .bind(id)
            .fetch_one(&mut admin)
            .await
            .expect("processlist")
    }

    /// Waits until the server dropped that connection.
    pub async fn wait_until_gone(&self, id: u64) {
        for _ in 0..50 {
            if self.connection_count(id).await == 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("connection {id} is still alive");
    }
}

pub async fn connect_admin(
    server: &Server,
    has_tls: bool,
    database: Option<&str>,
) -> MySqlConnection {
    let mut options = MySqlConnectOptions::new()
        .host("127.0.0.1")
        .port(server.port)
        .username("root")
        .password(PASSWORD)
        .ssl_mode(if has_tls {
            MySqlSslMode::Required
        } else {
            MySqlSslMode::Disabled
        });
    if let Some(database) = database {
        options = options.database(database);
    }
    // The readiness log line can precede the first accepted connection, so retry.
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        match MySqlConnection::connect_with(&options).await {
            Ok(connection) => return connection,
            Err(error) if Instant::now() > deadline => {
                panic!("mysql never became ready: {error}")
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
        }
    }
}
