#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::time::Duration;

use dbased_commands::Value;
use dbased_core::drivers::mysql::{MysqlConfig, MysqlDriver, TlsMode};
use dbased_core::{
    CancelOutcome, CloseReason, CoreError, Driver, DriverFactory, RowSink, Session, SessionHealth,
    SessionLimits, StatementStatus, TableRef, UserSql,
};
use secrecy::SecretString;
use support::mysql::TestDatabase;
use support::{run, run_with};
use tokio_util::sync::CancellationToken;

async fn open(database: &TestDatabase) -> (Box<dyn Driver>, Box<dyn Session>) {
    open_with(database.config()).await
}

async fn open_with(config: MysqlConfig) -> (Box<dyn Driver>, Box<dyn Session>) {
    let driver = MysqlDriver::connect(config).await.expect("connects");
    let session = driver.open_session().await.expect("opens a session");
    (driver, session)
}

fn capped_config(database: &TestDatabase, max_rows: u64) -> MysqlConfig {
    let mut config = database.config();
    config.limits = SessionLimits {
        max_rows,
        ..SessionLimits::default()
    };
    config
}

fn cancel_after(token: &CancellationToken, delay: Duration) {
    let token = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        token.cancel();
    });
}

#[tokio::test]
async fn session_id_is_the_connection_id() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;
    let id = session.session_id();

    assert_eq!(database.connection_count(id.0).await, 1);
    let result = run(&mut session, "SELECT CONNECTION_ID()").await;
    // BIGINT UNSIGNED fits i64 here.
    assert_eq!(
        result.rows,
        vec![vec![Value::Int(i64::try_from(id.0).unwrap())]]
    );
}

#[tokio::test]
async fn streams_a_multi_row_result_with_headers_first() {
    let database = TestDatabase::create().await;
    database.create_numbers(1000).await;
    let (_driver, mut session) = open(&database).await;

    let result = run(
        &mut session,
        "SELECT n, CONCAT('row ', n) AS label FROM nums ORDER BY n",
    )
    .await;

    assert_eq!(result.outcome.status, StatementStatus::Completed);
    assert_eq!(result.outcome.rows_received, 1000);
    assert_eq!(result.outcome.rows_affected, None);
    assert_eq!(result.rows.len(), 1000);
    assert_eq!(result.columns.len(), 2);
    assert_eq!(result.columns[0].name, "n");
    assert_eq!(result.columns[0].type_name, "INT");
    assert_eq!(result.columns[1].type_name, "VARCHAR");
    assert_eq!(result.rows[999][1], Value::Text("row 1000".into()));
}

#[tokio::test]
async fn headers_and_zero_rows_for_an_empty_select() {
    let database = TestDatabase::create().await;
    database.create_numbers(10).await;
    let (_driver, mut session) = open(&database).await;

    let result = run(&mut session, "SELECT n AS one FROM nums WHERE 1 = 0").await;

    assert_eq!(result.columns.len(), 1);
    assert_eq!(result.columns[0].name, "one");
    assert_eq!(result.columns[0].type_name, "INT");
    assert_eq!(result.outcome.rows_received, 0);
    assert_eq!(result.outcome.rows_affected, None);
    assert_eq!(result.outcome.status, StatementStatus::Completed);
}

#[tokio::test]
async fn rows_affected_is_reported_only_for_statements_without_rows() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;

    run(&mut session, "CREATE TEMPORARY TABLE t (id INT)").await;
    let inserted = run(&mut session, "INSERT INTO t VALUES (1), (2), (3)").await;

    assert_eq!(inserted.outcome.rows_affected, Some(3));
    assert!(inserted.columns.is_empty());
}

#[tokio::test]
async fn one_statement_per_call_extra_statements_are_flagged_and_not_streamed() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;

    let result = run(&mut session, "SELECT 1; SELECT 2").await;

    assert!(result.outcome.extra_statements_discarded);
    assert_eq!(result.rows, vec![vec![Value::Int(1)]]);
    let next = run(&mut session, "SELECT 3").await;
    assert_eq!(next.rows, vec![vec![Value::Int(3)]]);
    assert!(!next.outcome.extra_statements_discarded);
}

#[tokio::test]
async fn server_errors_carry_sqlstate_and_error_number() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;
    run(
        &mut session,
        "CREATE TEMPORARY TABLE u (id INT PRIMARY KEY)",
    )
    .await;
    run(&mut session, "INSERT INTO u VALUES (1)").await;

    let cases = [
        ("SELEC 1", "42000", "MySQL error 1064"),
        ("SELECT * FROM missing_table", "42S02", "MySQL error 1146"),
        ("INSERT INTO u VALUES (1)", "23000", "MySQL error 1062"),
        ("SELECT 1 / 0 + x", "42S22", "MySQL error 1054"),
    ];
    for (sql, sqlstate, detail) in cases {
        let result = run(&mut session, sql).await;
        assert_eq!(result.outcome.status, StatementStatus::Failed, "{sql}");
        let error = result.outcome.server_error.expect("server error");
        assert_eq!(error.sqlstate, sqlstate, "{sql}");
        assert_eq!(error.detail.as_deref(), Some(detail), "{sql}");
        assert_eq!(result.outcome.session, SessionHealth::Healthy, "{sql}");
    }
}

#[tokio::test]
async fn value_matrix() {
    let database = TestDatabase::create().await;
    database
        .setup(&[
            "CREATE TABLE matrix (
                big BIGINT UNSIGNED, small INT UNSIGNED, num DECIMAL(30,10), doc JSON,
                blobby BLOB, varbin VARBINARY(8), mood ENUM('sad','happy'), tags SET('a','b','c'),
                bits BIT(8), yr YEAR, tm TIME, d DATE, dt DATETIME, ts TIMESTAMP(1) NULL,
                flag TINYINT(1), dbl DOUBLE, fl FLOAT, txt VARCHAR(20), nul INT, zero_d DATE,
                zero_dt DATETIME)",
            "SET SESSION sql_mode = ''",
            "INSERT INTO matrix VALUES (
                18446744073709551615, 4294967295, 12345678901234567890.1234567890,
                '{\"a\":[1,2]}', x'DEADBEEF', x'0001', 'happy', 'a,c', b'00000101', 2024,
                '-838:59:59', '2024-03-05', '2024-03-05 10:20:30', '2024-03-05 10:20:30.5',
                1, 1.5, 2.5, 'héllo', NULL, '0000-00-00', '0000-00-00 00:00:00')",
        ])
        .await;
    let (_driver, mut session) = open(&database).await;

    let result = run(&mut session, "SELECT * FROM matrix").await;

    let row = &result.rows[0];
    assert_eq!(row[0], Value::Unknown("18446744073709551615".into()));
    assert_eq!(row[1], Value::Int(4_294_967_295));
    assert_eq!(
        row[2],
        Value::Decimal("12345678901234567890.1234567890".into())
    );
    assert_eq!(row[3], Value::Json("{\"a\": [1, 2]}".into()));
    assert_eq!(row[4], Value::Bytes(vec![0xde, 0xad, 0xbe, 0xef]));
    assert_eq!(row[5], Value::Bytes(vec![0, 1]));
    assert_eq!(row[6], Value::Text("happy".into()));
    assert_eq!(row[7], Value::Text("a,c".into()));
    assert_eq!(row[8], Value::Bytes(vec![5]));
    assert_eq!(row[9], Value::Int(2024));
    assert_eq!(row[10], Value::Time("-838:59:59".into()));
    assert_eq!(row[11], Value::Date("2024-03-05".into()));
    assert_eq!(row[12], Value::Timestamp("2024-03-05 10:20:30".into()));
    assert_eq!(row[13], Value::Timestamp("2024-03-05 10:20:30.5".into()));
    assert_eq!(row[14], Value::Bool(true));
    assert_eq!(row[15], Value::Float(1.5));
    assert_eq!(row[16], Value::Float(2.5));
    assert_eq!(row[17], Value::Text("héllo".into()));
    assert_eq!(row[18], Value::Null);
    // Zero-dates are shown as the server renders them, never as NULL.
    assert_eq!(row[19], Value::Date("0000-00-00".into()));
    assert_eq!(row[20], Value::Timestamp("0000-00-00 00:00:00".into()));
    let types: Vec<&str> = result
        .columns
        .iter()
        .map(|column| column.type_name.as_str())
        .collect();
    assert_eq!(types[0], "BIGINT UNSIGNED");
    assert_eq!(types[4], "BLOB");
    assert_eq!(types[6], "ENUM");
    assert_eq!(types[8], "BIT");
    assert_eq!(types[14], "BOOLEAN");
}

#[tokio::test]
async fn the_server_defaults_are_left_alone_on_connect() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;
    let mut admin = database.admin().await;
    let global_mode: String = sqlx::query_scalar("SELECT CAST(@@GLOBAL.sql_mode AS CHAR)")
        .fetch_one(&mut admin)
        .await
        .unwrap();
    let global_zone: String = sqlx::query_scalar("SELECT CAST(@@GLOBAL.time_zone AS CHAR)")
        .fetch_one(&mut admin)
        .await
        .unwrap();

    let result = run(&mut session, "SELECT @@sql_mode, @@time_zone").await;

    assert_eq!(
        result.rows,
        vec![vec![Value::Text(global_mode), Value::Text(global_zone)]]
    );
}

#[tokio::test]
async fn clean_cancel_keeps_the_session_and_its_temp_table() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;
    run(&mut session, "CREATE TEMPORARY TABLE kept (id INT)").await;
    run(&mut session, "INSERT INTO kept VALUES (42)").await;

    let token = CancellationToken::new();
    cancel_after(&token, Duration::from_millis(300));
    let started = std::time::Instant::now();
    let result = run_with(&mut session, "SELECT SLEEP(30)", token).await;

    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(result.outcome.status, StatementStatus::Cancelled);
    assert!(result.outcome.partial);
    // MySQL may report success (SLEEP returns 1) or error 1317 after KILL QUERY (3.2, rule 5).
    if let Some(error) = &result.outcome.server_error {
        assert_eq!(error.detail.as_deref(), Some("MySQL error 1317"));
    }
    assert_eq!(result.outcome.session, SessionHealth::Healthy);
    assert!(result.outcome.cancel_error.is_none());

    let after = run(&mut session, "SELECT id FROM kept").await;
    assert_eq!(after.rows, vec![vec![Value::Int(42)]]);
}

#[tokio::test]
async fn a_cancel_racing_the_end_of_the_statement_never_hits_the_next_one() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;

    for step in 0..40u64 {
        let token = CancellationToken::new();
        cancel_after(&token, Duration::from_millis(step * 2));
        let result = run_with(&mut session, "SELECT SLEEP(0.04)", token).await;
        assert!(
            matches!(
                result.outcome.status,
                StatementStatus::Cancelled | StatementStatus::Completed
            ),
            "step {step}: {:?}",
            result.outcome
        );
        assert_eq!(
            result.outcome.session,
            SessionHealth::Healthy,
            "step {step}"
        );

        let next = run(&mut session, "SELECT 7").await;
        assert_eq!(
            next.outcome.status,
            StatementStatus::Completed,
            "step {step}: {:?}",
            next.outcome
        );
        assert_eq!(next.rows, vec![vec![Value::Int(7)]]);
    }
}

#[tokio::test]
async fn a_user_cancel_inside_a_transaction_keeps_the_transaction_open() {
    let database = TestDatabase::create().await;
    database.setup(&["CREATE TABLE t (v INT)"]).await;
    let (driver, mut session) = open(&database).await;
    run(&mut session, "BEGIN").await;
    run(&mut session, "INSERT INTO t VALUES (1)").await;

    let token = CancellationToken::new();
    cancel_after(&token, Duration::from_millis(300));
    let cancelled = run_with(&mut session, "SELECT SLEEP(30)", token).await;
    assert_eq!(cancelled.outcome.status, StatementStatus::Cancelled);
    // MySQL has no aborted-transaction state: the user's work is still there.
    assert_eq!(cancelled.outcome.session, SessionHealth::Healthy);

    let commit = run(&mut session, "COMMIT").await;
    assert_eq!(commit.outcome.status, StatementStatus::Completed);
    let mut other = driver.open_session().await.unwrap();
    let count = run(&mut other, "SELECT COUNT(*) FROM t").await;
    assert_eq!(count.rows, vec![vec![Value::Int(1)]]);
}

#[tokio::test]
async fn row_cap_cancels_explicitly_drains_and_keeps_the_session() {
    let database = TestDatabase::create().await;
    database.create_numbers(2000).await;
    let (_driver, mut session) = open_with(capped_config(&database, 100)).await;

    let result = run(
        &mut session,
        "SELECT a.n FROM nums a CROSS JOIN nums b CROSS JOIN nums c",
    )
    .await;

    assert_eq!(result.outcome.status, StatementStatus::Truncated);
    assert!(result.outcome.partial);
    assert_eq!(result.outcome.rows_received, 100);
    assert_eq!(result.rows.len(), 100);
    assert_eq!(result.outcome.session, SessionHealth::Healthy);
    let next = run(&mut session, "SELECT 1").await;
    assert_eq!(next.outcome.status, StatementStatus::Completed);
}

#[tokio::test]
async fn a_result_exactly_at_the_row_cap_is_not_truncated() {
    let database = TestDatabase::create().await;
    database.create_numbers(10).await;
    let (_driver, mut session) = open_with(capped_config(&database, 10)).await;

    let result = run(&mut session, "SELECT n FROM nums").await;

    assert_eq!(result.outcome.status, StatementStatus::Completed);
    assert_eq!(result.outcome.rows_received, 10);
}

#[tokio::test]
async fn memory_cap_truncates_too() {
    let database = TestDatabase::create().await;
    database.create_numbers(2000).await;
    let mut config = database.config();
    config.limits.max_bytes = 4096;
    let (_driver, mut session) = open_with(config).await;

    let result = run(
        &mut session,
        "SELECT REPEAT('x', 1000) FROM nums a CROSS JOIN nums b",
    )
    .await;

    assert_eq!(result.outcome.status, StatementStatus::Truncated);
    assert!(result.outcome.rows_received >= 1 && result.outcome.rows_received < 100);
    assert_eq!(result.outcome.session, SessionHealth::Healthy);
}

#[tokio::test]
async fn a_row_cap_inside_a_transaction_keeps_the_users_update() {
    let database = TestDatabase::create().await;
    database.create_numbers(500).await;
    database
        .setup(&[
            "CREATE TABLE t (id INT, v INT)",
            "INSERT INTO t VALUES (1, 1)",
        ])
        .await;
    let (driver, mut session) = open_with(capped_config(&database, 100)).await;

    run(&mut session, "BEGIN").await;
    let updated = run(&mut session, "UPDATE t SET v = 2").await;
    assert_eq!(updated.outcome.status, StatementStatus::Completed);
    let capped = run(&mut session, "SELECT a.n FROM nums a CROSS JOIN nums b").await;

    assert_eq!(capped.outcome.status, StatementStatus::Truncated);
    assert!(capped.outcome.partial);
    assert_eq!(capped.rows.len(), 100);
    assert!(capped.outcome.cancel_error.is_none());
    // @NOTE: nothing was killed: the rows were read and discarded.
    assert!(capped.outcome.server_error.is_none(), "{capped:?}");
    assert_eq!(capped.outcome.session, SessionHealth::Healthy);
    let commit = run(&mut session, "COMMIT").await;
    assert_eq!(commit.outcome.status, StatementStatus::Completed);

    let mut other = driver.open_session().await.unwrap();
    let value = run(&mut other, "SELECT v FROM t").await;
    assert_eq!(value.rows, vec![vec![Value::Int(2)]]);
}

#[tokio::test]
async fn a_row_cap_outside_a_transaction_cancels_after_every_way_a_transaction_ends() {
    let database = TestDatabase::create().await;
    database.create_numbers(2000).await;
    database.setup(&["CREATE TABLE t (v INT)"]).await;
    let (_driver, mut session) = open_with(capped_config(&database, 10)).await;
    let endings = [
        "COMMIT",
        "ROLLBACK",
        "CREATE TABLE implicit_commit (a INT)",
        "SET autocommit = 1",
    ];
    for ending in endings {
        run(&mut session, "BEGIN").await;
        run(&mut session, "INSERT INTO t VALUES (1)").await;
        let closing = run(&mut session, ending).await;
        assert_eq!(
            closing.outcome.status,
            StatementStatus::Completed,
            "{ending}"
        );
        run(&mut session, "DROP TABLE IF EXISTS implicit_commit").await;
        let started = std::time::Instant::now();
        let capped = run(
            &mut session,
            "SELECT a.n FROM nums a CROSS JOIN nums b CROSS JOIN nums c",
        )
        .await;
        assert_eq!(
            capped.outcome.status,
            StatementStatus::Truncated,
            "{ending}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{ending}: the cap should have killed the query instead of draining billions of rows"
        );
    }
}

#[tokio::test]
async fn autocommit_off_protects_the_open_transaction_from_a_cap() {
    let database = TestDatabase::create().await;
    database.create_numbers(500).await;
    database.setup(&["CREATE TABLE t (v INT)"]).await;
    let (driver, mut session) = open_with(capped_config(&database, 10)).await;

    run(&mut session, "SET autocommit = 0").await;
    run(&mut session, "INSERT INTO t VALUES (1)").await;
    let capped = run(&mut session, "SELECT a.n FROM nums a CROSS JOIN nums b").await;
    assert_eq!(capped.outcome.status, StatementStatus::Truncated);
    assert!(capped.outcome.server_error.is_none(), "{capped:?}");
    run(&mut session, "COMMIT").await;

    let mut other = driver.open_session().await.unwrap();
    let count = run(&mut other, "SELECT COUNT(*) FROM t").await;
    assert_eq!(count.rows, vec![vec![Value::Int(1)]]);
}

#[tokio::test]
async fn the_transaction_probe_leaves_the_users_transaction_settings_alone() {
    let database = TestDatabase::create().await;
    database.setup(&["CREATE TABLE t (v INT)"]).await;
    let (_driver, mut session) = open(&database).await;

    // A pending one-shot setting must survive the probe that BEGIN triggers.
    run(&mut session, "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE").await;
    run(&mut session, "BEGIN").await;
    run(&mut session, "SELECT * FROM t").await;
    let level = run(
        &mut session,
        "SELECT trx_isolation_level FROM information_schema.INNODB_TRX \
         WHERE trx_mysql_thread_id = CONNECTION_ID()",
    )
    .await;
    assert_eq!(level.rows, vec![vec![Value::Text("SERIALIZABLE".into())]]);
    run(&mut session, "ROLLBACK").await;

    // A session default of READ ONLY must survive the probe that COMMIT triggers.
    run(&mut session, "SET SESSION TRANSACTION READ ONLY").await;
    run(&mut session, "BEGIN").await;
    run(&mut session, "COMMIT").await;
    run(&mut session, "BEGIN").await;
    let insert = run(&mut session, "INSERT INTO t VALUES (1)").await;
    let error = insert.outcome.server_error.expect("read-only transaction");
    assert_eq!(error.detail.as_deref(), Some("MySQL error 1792"));
}

#[tokio::test]
async fn a_dropped_execute_future_poisons_the_session() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;
    let id = session.session_id();

    let (sink, _receiver) = RowSink::channel(1);
    let execution = session.execute(
        UserSql::new("SELECT SLEEP(30)"),
        sink,
        CancellationToken::new(),
    );
    let timed_out = tokio::time::timeout(Duration::from_millis(200), execution).await;
    assert!(timed_out.is_err());

    assert_eq!(
        session.health_check().await.unwrap(),
        SessionHealth::Closed(CloseReason::FutureDropped)
    );
    let (sink, _receiver) = RowSink::channel(1);
    let next = session
        .execute(UserSql::new("SELECT 1"), sink, CancellationToken::new())
        .await;
    assert!(matches!(
        next,
        Err(CoreError::SessionClosed(CloseReason::FutureDropped))
    ));

    // The abandoned statement was killed and the connection closed on release.
    database.wait_until_gone(id.0).await;
}

#[tokio::test]
async fn a_token_cancelled_before_execute_never_sends_the_statement() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;

    let token = CancellationToken::new();
    token.cancel();
    let started = std::time::Instant::now();
    let result = run_with(&mut session, "SELECT SLEEP(30)", token).await;

    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(result.outcome.status, StatementStatus::Cancelled);
    assert_eq!(result.outcome.rows_received, 0);
    assert!(result.columns.is_empty());
    assert_eq!(result.outcome.session, SessionHealth::Healthy);
    let next = run(&mut session, "SELECT 1").await;
    assert_eq!(next.rows, vec![vec![Value::Int(1)]]);
}

#[tokio::test]
async fn a_full_channel_with_a_cancel_does_not_block_execute() {
    let database = TestDatabase::create().await;
    database.create_numbers(2000).await;
    let (_driver, mut session) = open(&database).await;

    // Nobody reads, so the channel fills up and the driver waits on the sink.
    let (sink, receiver) = RowSink::channel(1);
    let token = CancellationToken::new();
    cancel_after(&token, Duration::from_millis(300));
    let started = std::time::Instant::now();
    let outcome = session
        .execute(
            UserSql::new("SELECT a.n FROM nums a CROSS JOIN nums b CROSS JOIN nums c"),
            sink,
            token,
        )
        .await
        .unwrap();

    assert!(started.elapsed() < Duration::from_secs(15));
    assert_eq!(outcome.status, StatementStatus::Cancelled);
    assert!(outcome.rows_received <= 1, "{outcome:?}");
    assert_eq!(outcome.session, SessionHealth::Healthy);
    drop(receiver);
    let next = run(&mut session, "SELECT 1").await;
    assert_eq!(next.outcome.status, StatementStatus::Completed);
}

#[tokio::test]
async fn a_dropped_receiver_cancels_with_consumer_gone_semantics() {
    let database = TestDatabase::create().await;
    database.create_numbers(2000).await;
    let (_driver, mut session) = open(&database).await;

    let (sink, receiver) = RowSink::channel(1);
    drop(receiver);
    let outcome = session
        .execute(
            UserSql::new("SELECT a.n FROM nums a CROSS JOIN nums b CROSS JOIN nums c"),
            sink,
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(outcome.status, StatementStatus::Cancelled);
    assert!(outcome.partial);
    assert_eq!(outcome.rows_received, 0);
    assert_eq!(outcome.session, SessionHealth::Healthy);
    let next = run(&mut session, "SELECT 1").await;
    assert_eq!(next.outcome.status, StatementStatus::Completed);
}

#[tokio::test]
async fn cancel_works_when_the_pool_is_busy_and_falls_back_to_a_one_off_connection() {
    let database = TestDatabase::create().await;
    let mut config = database.config();
    config.pool_max_connections = 1;
    config.cancel_acquire_timeout = Duration::from_millis(1);
    let driver = MysqlDriver::connect(config).await.expect("connects");
    let mut session = driver.open_session().await.unwrap();

    // Keep the single pool connection busy so acquiring it for the cancel times out.
    let driver: std::sync::Arc<dyn Driver> = driver.into();
    let busy: Vec<_> = (0..8)
        .map(|_| {
            let driver = driver.clone();
            tokio::spawn(async move {
                loop {
                    driver.list_schemas().await.ok();
                }
            })
        })
        .collect();

    let token = CancellationToken::new();
    cancel_after(&token, Duration::from_millis(300));
    let started = std::time::Instant::now();
    let result = run_with(&mut session, "SELECT SLEEP(30)", token).await;
    for task in busy {
        task.abort();
    }

    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(result.outcome.status, StatementStatus::Cancelled);
    assert!(
        result.outcome.cancel_error.is_none(),
        "{:?}",
        result.outcome
    );
    assert_eq!(result.outcome.session, SessionHealth::Healthy);
}

#[tokio::test]
async fn a_server_side_kill_closes_the_session() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;
    let id = session.session_id();

    let mut admin = database.admin().await;
    let kill = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("KILL CONNECTION {}", id.0)))
            .execute(&mut admin)
            .await
            .unwrap();
    };
    let (sink, _receiver) = RowSink::channel(8);
    let (result, ()) = tokio::join!(
        session.execute(
            UserSql::new("SELECT SLEEP(30)"),
            sink,
            CancellationToken::new()
        ),
        kill
    );

    // The server reports an error (outcome) or the socket just dies: both end the session.
    if let Ok(outcome) = result {
        assert_eq!(
            outcome.session,
            SessionHealth::Closed(CloseReason::ConnectionError)
        );
    }
    let (sink, _receiver) = RowSink::channel(1);
    let next = session
        .execute(UserSql::new("SELECT 1"), sink, CancellationToken::new())
        .await;
    assert!(matches!(next, Err(CoreError::SessionClosed(_))));
}

#[tokio::test]
async fn closing_a_session_ends_the_connection() {
    let database = TestDatabase::create().await;
    let (_driver, session) = open(&database).await;
    let id = session.session_id();
    session.close().await;

    database.wait_until_gone(id.0).await;
}

#[tokio::test]
async fn the_cancel_handle_cancels_a_running_statement() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;
    let handle = session.cancel_handle();

    let cancelling = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        handle.cancel().await
    });
    let started = std::time::Instant::now();
    let result = run(&mut session, "SELECT SLEEP(30)").await;

    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(cancelling.await.unwrap().unwrap(), CancelOutcome::Sent);
    assert_eq!(result.outcome.session, SessionHealth::Healthy);
    let next = run(&mut session, "SELECT 1").await;
    assert_eq!(next.rows, vec![vec![Value::Int(1)]]);
}

#[tokio::test]
async fn a_cancel_handle_after_close_is_a_harmless_no_op() {
    let database = TestDatabase::create().await;
    let (driver, session) = open(&database).await;
    let handle = session.cancel_handle();
    let clone = handle.clone();
    session.close().await;

    assert_eq!(handle.cancel().await.unwrap(), CancelOutcome::SessionClosed);
    assert_eq!(clone.cancel().await.unwrap(), CancelOutcome::SessionClosed);

    // A stranger session keeps running; the closed handle never reaches it.
    let mut stranger = driver.open_session().await.unwrap();
    let result = run(&mut stranger, "SELECT SLEEP(0.5), 1").await;
    assert_eq!(result.outcome.status, StatementStatus::Completed);
    assert_eq!(result.rows, vec![vec![Value::Int(0), Value::Int(1)]]);
}

#[tokio::test]
async fn wrong_password_is_an_authentication_error() {
    let database = TestDatabase::create().await;
    let mut config = database.config();
    config.password = SecretString::from("wrong");

    let error = MysqlDriver::connect(config).await.err().expect("fails");

    assert_eq!(error.kind(), "authentication", "{error}");
}

#[tokio::test]
async fn unreachable_host_is_a_connection_error() {
    let database = TestDatabase::create().await;
    let mut config = database.config();
    config.port = 1;

    let error = MysqlDriver::connect(config).await.err().expect("fails");

    assert_eq!(error.kind(), "connection", "{error}");
}

#[tokio::test]
async fn an_unknown_database_is_a_database_error() {
    let database = TestDatabase::create().await;
    let mut config = database.config();
    config.database = "no_such_database".into();

    let error = MysqlDriver::connect(config).await.err().expect("fails");

    assert_eq!(error.kind(), "database", "{error}");
}

#[tokio::test]
async fn require_fails_clearly_against_a_server_without_tls() {
    let database = TestDatabase::create_without_tls().await;
    let mut config = database.config();
    config.tls = TlsMode::Require;

    let error = MysqlDriver::connect(config).await.err().expect("fails");

    assert_eq!(error.kind(), "tls", "{error}");
}

#[tokio::test]
async fn require_encrypts_the_connection_when_the_server_offers_tls() {
    let database = TestDatabase::create().await;
    assert_eq!(
        MysqlConfig::new("h", 1, "u", SecretString::from("p"), "d").tls,
        TlsMode::Require
    );
    let (_driver, mut session) = open(&database).await;

    let result = run(&mut session, "SHOW SESSION STATUS LIKE 'Ssl_cipher'").await;

    let Value::Text(cipher) = &result.rows[0][1] else {
        panic!("a text cipher name: {:?}", result.rows);
    };
    assert!(!cipher.is_empty());
}

#[tokio::test]
async fn prefer_is_an_explicit_opt_in_that_falls_back_to_plaintext() {
    let database = TestDatabase::create_without_tls().await;
    let mut config = database.config();
    config.tls = TlsMode::Prefer;

    let (_driver, mut session) = open_with(config).await;

    assert_eq!(
        run(&mut session, "SELECT 1").await.rows,
        vec![vec![Value::Int(1)]]
    );
}

#[tokio::test]
async fn metadata_lists_schemas_tables_views_and_columns() {
    let database = TestDatabase::create().await;
    database
        .setup(&[
            "CREATE TABLE people (id INT UNSIGNED NOT NULL, name VARCHAR(40))",
            "CREATE VIEW adults AS SELECT * FROM people",
        ])
        .await;
    let (driver, _session) = open(&database).await;

    let schemas = driver.list_schemas().await.unwrap();
    assert!(schemas.iter().any(|schema| schema.name == database.name));
    assert!(schemas.iter().all(|schema| schema.name != "mysql"));
    assert!(
        schemas
            .iter()
            .all(|schema| schema.name != "information_schema")
    );
    let tables = driver.list_tables(&database.name).await.unwrap();
    assert_eq!(
        tables,
        vec![TableRef {
            schema: database.name.clone(),
            name: "people".into()
        }]
    );
    let views = driver.list_views(&database.name).await.unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].name, "adults");
    let columns = driver.list_columns(&tables[0]).await.unwrap();
    assert_eq!(columns.len(), 2);
    assert_eq!(
        (columns[0].name.as_str(), columns[0].nullable),
        ("id", false)
    );
    assert_eq!(columns[0].data_type, "int unsigned");
    assert_eq!(columns[1].data_type, "varchar(40)");
    assert!(columns[1].nullable);
    assert!(matches!(
        driver.list_routines(&database.name).await,
        Err(CoreError::NotImplemented(_))
    ));
}
