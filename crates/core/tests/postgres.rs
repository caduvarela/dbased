#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::time::Duration;

use dbased_commands::Value;
use dbased_core::drivers::postgres::{PostgresDriver, TlsMode};
use dbased_core::{
    CancelOutcome, CloseReason, CoreError, Driver, DriverFactory, RowSink, Session, SessionHealth,
    SessionLimits, StatementStatus, TableRef, UserSql,
};
use secrecy::SecretString;
use sqlx::Connection as _;
use support::{TestDatabase, run, run_with};
use tokio_util::sync::CancellationToken;

async fn open(database: &TestDatabase) -> (Box<dyn Driver>, Box<dyn Session>) {
    open_with(database.config()).await
}

async fn open_with(
    config: dbased_core::drivers::postgres::PostgresConfig,
) -> (Box<dyn Driver>, Box<dyn Session>) {
    let driver = PostgresDriver::connect(config).await.expect("connects");
    let session = driver.open_session().await.expect("opens a session");
    (driver, session)
}

fn cancel_after(token: &CancellationToken, delay: Duration) {
    let token = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        token.cancel();
    });
}

#[tokio::test]
async fn session_id_is_the_backend_pid() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;
    let id = session.session_id();

    let mut admin = database.admin().await;
    let known: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE pid = $1")
        .bind(i32::try_from(id.0).unwrap())
        .fetch_one(&mut admin)
        .await
        .unwrap();
    assert_eq!(known, 1);

    let result = run(&mut session, "SELECT pg_backend_pid()").await;
    assert_eq!(
        result.rows,
        vec![vec![Value::Int(i64::try_from(id.0).unwrap())]]
    );
}

#[tokio::test]
async fn streams_a_multi_row_result_with_headers_first() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;

    let result = run(
        &mut session,
        "SELECT g AS n, 'row ' || g AS label FROM generate_series(1, 1000) g",
    )
    .await;

    assert_eq!(result.outcome.status, StatementStatus::Completed);
    assert_eq!(result.outcome.rows_received, 1000);
    assert_eq!(result.outcome.rows_affected, None);
    assert_eq!(result.rows.len(), 1000);
    assert_eq!(result.columns.len(), 2);
    assert_eq!(result.columns[0].name, "n");
    assert_eq!(result.columns[0].type_name, "INT4");
    assert_eq!(result.rows[999][1], Value::Text("row 1000".into()));
}

#[tokio::test]
async fn headers_and_zero_rows_for_an_empty_select() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;

    let result = run(&mut session, "SELECT 1 AS one WHERE false").await;

    assert_eq!(result.columns.len(), 1);
    assert_eq!(result.columns[0].name, "one");
    assert_eq!(result.outcome.rows_received, 0);
    assert_eq!(result.outcome.rows_affected, None);
}

#[tokio::test]
async fn rows_affected_is_reported_only_for_statements_without_rows() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;

    run(&mut session, "CREATE TEMP TABLE t (id int)").await;
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
}

#[tokio::test]
async fn syntax_errors_carry_sqlstate_and_position() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;

    let result = run(&mut session, "SELEC 1").await;

    assert_eq!(result.outcome.status, StatementStatus::Failed);
    let error = result.outcome.server_error.expect("server error");
    assert_eq!(error.sqlstate, "42601");
    assert_eq!(error.position, Some(1));
    assert_eq!(result.outcome.session, SessionHealth::Healthy);
}

#[tokio::test]
async fn value_matrix() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;
    run(&mut session, "CREATE TYPE mood AS ENUM ('sad', 'happy')").await;
    run(&mut session, "CREATE TYPE pair AS (a int, b text)").await;

    let result = run(
        &mut session,
        "SELECT 12345678901234567890.123456789::numeric AS num, \
                '2024-03-05 10:20:30.5+02'::timestamptz AS tz, \
                interval '1 day 2 hours' AS iv, \
                ARRAY[1,2,3]::int4[] AS arr, \
                '{\"a\":[1,2]}'::jsonb AS j, \
                '\\xdeadbeef'::bytea AS b, \
                '123e4567-e89b-12d3-a456-426614174000'::uuid AS u, \
                'happy'::mood AS e, \
                ROW(1,'x')::pair AS c, \
                NULL::int AS nul, \
                true AS flag, \
                1.5::float8 AS f, \
                7::int2 AS small, \
                'abc'::text AS t, \
                DATE '2024-03-05' AS d, \
                TIME '10:20:30' AS tm",
    )
    .await;

    let row = &result.rows[0];
    assert_eq!(
        row[0],
        Value::Decimal("12345678901234567890.123456789".into())
    );
    assert_eq!(row[1], Value::Timestamp("2024-03-05 08:20:30.5+00".into()));
    assert_eq!(row[2], Value::Unknown("1 day 02:00:00".into()));
    assert_eq!(row[3], Value::Unknown("{1,2,3}".into()));
    assert_eq!(row[4], Value::Json("{\"a\": [1, 2]}".into()));
    assert_eq!(row[5], Value::Bytes(vec![0xde, 0xad, 0xbe, 0xef]));
    assert_eq!(
        row[6],
        Value::Uuid("123e4567-e89b-12d3-a456-426614174000".into())
    );
    assert_eq!(row[7], Value::Unknown("happy".into()));
    assert_eq!(row[8], Value::Unknown("(1,x)".into()));
    assert_eq!(row[9], Value::Null);
    assert_eq!(row[10], Value::Bool(true));
    assert_eq!(row[11], Value::Float(1.5));
    assert_eq!(row[12], Value::Int(7));
    assert_eq!(row[13], Value::Text("abc".into()));
    assert_eq!(row[14], Value::Date("2024-03-05".into()));
    assert_eq!(row[15], Value::Time("10:20:30".into()));
    // Enum and composite OIDs are resolved through pg_type (docs/PROJECT.md 2.5).
    assert_eq!(result.columns[7].type_name, "mood");
    assert_eq!(result.columns[8].type_name, "pair");
}

#[tokio::test]
async fn clean_cancel_keeps_the_session_and_its_temp_table() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;
    run(&mut session, "CREATE TEMP TABLE kept (id int)").await;
    run(&mut session, "INSERT INTO kept VALUES (42)").await;

    let token = CancellationToken::new();
    cancel_after(&token, Duration::from_millis(300));
    let started = std::time::Instant::now();
    let result = run_with(&mut session, "SELECT pg_sleep(30)", token).await;

    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(result.outcome.status, StatementStatus::Cancelled);
    assert!(result.outcome.partial);
    assert_eq!(
        result.outcome.server_error.as_ref().unwrap().sqlstate,
        "57014"
    );
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
        let result = run_with(&mut session, "SELECT pg_sleep(0.04)", token).await;
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
async fn cancel_inside_a_transaction_reports_aborted_and_rollback_recovers() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;
    run(&mut session, "BEGIN").await;

    let token = CancellationToken::new();
    cancel_after(&token, Duration::from_millis(300));
    let cancelled = run_with(&mut session, "SELECT pg_sleep(30)", token).await;
    assert_eq!(cancelled.outcome.status, StatementStatus::Cancelled);
    assert_eq!(cancelled.outcome.session, SessionHealth::TransactionAborted);

    let blocked = run(&mut session, "SELECT 1").await;
    let error = blocked.outcome.server_error.expect("25P02");
    assert_eq!(error.sqlstate, "25P02");
    assert_eq!(blocked.outcome.session, SessionHealth::TransactionAborted);
    assert_eq!(
        session.health_check().await.unwrap(),
        SessionHealth::TransactionAborted
    );

    let rollback = run(&mut session, "ROLLBACK").await;
    assert_eq!(rollback.outcome.status, StatementStatus::Completed);
    let alive = run(&mut session, "SELECT 1").await;
    assert_eq!(alive.rows, vec![vec![Value::Int(1)]]);
}

#[tokio::test]
async fn drain_timeout_closes_the_session() {
    let database = TestDatabase::create().await;
    let mut config = database.config();
    config.drain_timeout = Duration::from_millis(400);
    let (_driver, mut session) = open_with(config).await;

    // A loop that swallows every cancel, so the statement never reaches its end in time.
    let stubborn = "DO $$ BEGIN FOR i IN 1..40 LOOP BEGIN PERFORM pg_sleep(0.25); \
                    EXCEPTION WHEN query_canceled THEN NULL; END; END LOOP; END $$";
    let token = CancellationToken::new();
    cancel_after(&token, Duration::from_millis(200));
    let result = run_with(&mut session, stubborn, token).await;

    assert_eq!(result.outcome.status, StatementStatus::Cancelled);
    assert_eq!(
        result.outcome.session,
        SessionHealth::Closed(CloseReason::DrainTimeout)
    );
    let (sink, _receiver) = RowSink::channel(1);
    let next = session
        .execute(UserSql::new("SELECT 1"), sink, CancellationToken::new())
        .await;
    assert!(matches!(
        next,
        Err(CoreError::SessionClosed(CloseReason::DrainTimeout))
    ));
}

#[tokio::test]
async fn row_cap_cancels_explicitly_drains_and_keeps_the_session() {
    let database = TestDatabase::create().await;
    let mut config = database.config();
    config.limits = SessionLimits {
        max_rows: 100,
        ..SessionLimits::default()
    };
    let (_driver, mut session) = open_with(config).await;

    let result = run(&mut session, "SELECT g FROM generate_series(1, 5000000) g").await;

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
    let mut config = database.config();
    config.limits.max_rows = 10;
    let (_driver, mut session) = open_with(config).await;

    let result = run(&mut session, "SELECT g FROM generate_series(1, 10) g").await;

    assert_eq!(result.outcome.status, StatementStatus::Completed);
    assert_eq!(result.outcome.rows_received, 10);
}

#[tokio::test]
async fn memory_cap_truncates_too() {
    let database = TestDatabase::create().await;
    let mut config = database.config();
    config.limits.max_bytes = 4096;
    let (_driver, mut session) = open_with(config).await;

    let result = run(
        &mut session,
        "SELECT repeat('x', 1000) FROM generate_series(1, 100000)",
    )
    .await;

    assert_eq!(result.outcome.status, StatementStatus::Truncated);
    assert!(result.outcome.rows_received >= 1 && result.outcome.rows_received < 100);
    assert_eq!(result.outcome.session, SessionHealth::Healthy);
}

#[tokio::test]
async fn a_dropped_execute_future_poisons_the_session() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;
    let id = session.session_id();

    let (sink, _receiver) = RowSink::channel(1);
    let execution = session.execute(
        UserSql::new("SELECT pg_sleep(30)"),
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

    // The abandoned statement was cancelled and the connection closed on release.
    let mut admin = database.admin().await;
    for _ in 0..40 {
        let alive: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE pid = $1")
            .bind(i32::try_from(id.0).unwrap())
            .fetch_one(&mut admin)
            .await
            .unwrap();
        if alive == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the abandoned backend is still alive");
}

#[tokio::test]
async fn a_token_cancelled_before_execute_never_sends_the_statement() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;

    let token = CancellationToken::new();
    token.cancel();
    let started = std::time::Instant::now();
    let result = run_with(&mut session, "SELECT pg_sleep(30)", token).await;

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
    let (_driver, mut session) = open(&database).await;

    // Nobody reads, so the channel fills up and the driver waits on the sink.
    let (sink, receiver) = RowSink::channel(1);
    let token = CancellationToken::new();
    cancel_after(&token, Duration::from_millis(300));
    let started = std::time::Instant::now();
    let outcome = session
        .execute(
            UserSql::new("SELECT g FROM generate_series(1, 5000000) g"),
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
    let (_driver, mut session) = open(&database).await;

    let (sink, receiver) = RowSink::channel(1);
    drop(receiver);
    let outcome = session
        .execute(
            UserSql::new("SELECT g FROM generate_series(1, 5000000) g"),
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
    let driver = PostgresDriver::connect(config).await.expect("connects");
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
    let result = run_with(&mut session, "SELECT pg_sleep(30)", token).await;
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
async fn a_server_side_termination_closes_the_session() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;
    let id = session.session_id();

    let mut admin = database.admin().await;
    let terminate = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        sqlx::query("SELECT pg_terminate_backend($1)")
            .bind(i32::try_from(id.0).unwrap())
            .execute(&mut admin)
            .await
            .unwrap();
    };
    let (sink, _receiver) = RowSink::channel(8);
    let (result, ()) = tokio::join!(
        session.execute(
            UserSql::new("SELECT pg_sleep(30)"),
            sink,
            CancellationToken::new()
        ),
        terminate
    );

    // The server reports 57P01 (outcome) or the socket just dies (error): both end the session.
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
async fn closing_a_session_ends_the_backend() {
    let database = TestDatabase::create().await;
    let (_driver, session) = open(&database).await;
    let id = session.session_id();
    session.close().await;

    let mut admin = database.admin().await;
    for _ in 0..40 {
        let alive: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE pid = $1")
            .bind(i32::try_from(id.0).unwrap())
            .fetch_one(&mut admin)
            .await
            .unwrap();
        if alive == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the backend survived close");
}

#[tokio::test]
async fn wrong_password_is_an_authentication_error() {
    let database = TestDatabase::create().await;
    let mut config = database.config();
    config.password = SecretString::from("wrong");

    let error = PostgresDriver::connect(config).await.err().expect("fails");

    assert_eq!(error.kind(), "authentication", "{error}");
}

#[tokio::test]
async fn unreachable_host_is_a_connection_error() {
    let database = TestDatabase::create().await;
    let mut config = database.config();
    config.port = 1;

    let error = PostgresDriver::connect(config).await.err().expect("fails");

    assert_eq!(error.kind(), "connection", "{error}");
}

#[tokio::test]
async fn require_fails_clearly_against_a_server_without_tls() {
    let database = TestDatabase::create().await;
    let mut config = database.config();
    config.tls = TlsMode::Require;

    let error = PostgresDriver::connect(config).await.err().expect("fails");

    assert_eq!(error.kind(), "tls", "{error}");
}

#[tokio::test]
async fn prefer_is_an_explicit_opt_in_that_falls_back_to_plaintext() {
    let database = TestDatabase::create().await;
    let mut config = database.config();
    config.tls = TlsMode::Prefer;

    let (_driver, mut session) = open_with(config).await;

    assert_eq!(
        run(&mut session, "SELECT 1").await.rows,
        vec![vec![Value::Int(1)]]
    );
}

#[tokio::test]
async fn metadata_lists_schemas_tables_and_columns() {
    let database = TestDatabase::create().await;
    let (driver, mut session) = open(&database).await;
    run(
        &mut session,
        "CREATE TABLE people (id int NOT NULL, name text)",
    )
    .await;
    run(&mut session, "CREATE VIEW adults AS SELECT * FROM people").await;

    let schemas = driver.list_schemas().await.unwrap();
    assert!(schemas.iter().any(|schema| schema.name == "public"));
    let tables = driver.list_tables("public").await.unwrap();
    assert_eq!(
        tables,
        vec![TableRef {
            schema: "public".into(),
            name: "people".into()
        }]
    );
    let views = driver.list_views("public").await.unwrap();
    assert_eq!(views.len(), 1);
    let columns = driver.list_columns(&tables[0]).await.unwrap();
    assert_eq!(columns.len(), 2);
    assert_eq!(
        (columns[0].name.as_str(), columns[0].nullable),
        ("id", false)
    );
    assert_eq!(columns[1].data_type, "text");
    assert!(matches!(
        driver.list_routines("public").await,
        Err(CoreError::NotImplemented(_))
    ));
}

fn capped_config(
    database: &TestDatabase,
    max_rows: u64,
) -> dbased_core::drivers::postgres::PostgresConfig {
    let mut config = database.config();
    config.limits = SessionLimits {
        max_rows,
        ..SessionLimits::default()
    };
    config
}

#[tokio::test]
async fn a_row_cap_inside_a_transaction_keeps_the_users_update() {
    let database = TestDatabase::create().await;
    let (driver, mut session) = open_with(capped_config(&database, 100)).await;
    run(&mut session, "CREATE TABLE t (id int, v int)").await;
    run(&mut session, "INSERT INTO t VALUES (1, 1)").await;

    run(&mut session, "BEGIN").await;
    let updated = run(&mut session, "UPDATE t SET v = 2").await;
    assert_eq!(updated.outcome.status, StatementStatus::Completed);
    let capped = run(&mut session, "SELECT g FROM generate_series(1, 300000) g").await;

    assert_eq!(capped.outcome.status, StatementStatus::Truncated);
    assert!(capped.outcome.partial);
    assert_eq!(capped.rows.len(), 100);
    assert!(capped.outcome.cancel_error.is_none());
    assert_eq!(capped.outcome.session, SessionHealth::Healthy);
    let commit = run(&mut session, "COMMIT").await;
    assert_eq!(commit.outcome.status, StatementStatus::Completed);

    let other = driver.open_session().await.unwrap();
    let mut other = other;
    let value = run(&mut other, "SELECT v FROM t").await;
    assert_eq!(value.rows, vec![vec![Value::Int(2)]]);
}

#[tokio::test]
async fn a_row_cap_outside_a_transaction_still_cancels_after_commit_and_rollback() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open_with(capped_config(&database, 10)).await;
    for closing in ["COMMIT", "ROLLBACK", "END", "ABORT"] {
        run(&mut session, "BEGIN").await;
        run(&mut session, closing).await;
        let started = std::time::Instant::now();
        let capped = run(
            &mut session,
            "SELECT g FROM generate_series(1, 100000000) g",
        )
        .await;
        assert_eq!(
            capped.outcome.status,
            StatementStatus::Truncated,
            "{closing}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{closing}: the cap should have cancelled instead of draining 100M rows"
        );
    }
}

#[tokio::test]
async fn the_transaction_state_survives_begin_probing_over_many_rounds() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open_with(capped_config(&database, 5)).await;
    run(&mut session, "CREATE TABLE t (v int)").await;
    for round in 0..25 {
        run(&mut session, "BEGIN").await;
        run(&mut session, "INSERT INTO t VALUES (1)").await;
        let capped = run(&mut session, "SELECT g FROM generate_series(1, 2000) g").await;
        assert_eq!(capped.outcome.status, StatementStatus::Truncated, "{round}");
        assert_eq!(capped.outcome.session, SessionHealth::Healthy, "{round}");
        run(&mut session, "COMMIT").await;
    }
    let count = run(&mut session, "SELECT count(*) FROM t").await;
    assert_eq!(count.rows, vec![vec![Value::Int(25)]]);
}

#[tokio::test]
async fn a_dropped_receiver_inside_a_transaction_keeps_the_users_update() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;
    run(&mut session, "CREATE TABLE t (v int)").await;
    run(&mut session, "BEGIN").await;
    run(&mut session, "INSERT INTO t VALUES (1)").await;

    let (sink, receiver) = RowSink::channel(1);
    drop(receiver);
    let outcome = session
        .execute(
            UserSql::new("SELECT g FROM generate_series(1, 200000) g"),
            sink,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(outcome.partial);
    assert_eq!(outcome.session, SessionHealth::Healthy);

    run(&mut session, "COMMIT").await;
    let count = run(&mut session, "SELECT count(*) FROM t").await;
    assert_eq!(count.rows, vec![vec![Value::Int(1)]]);
}

#[tokio::test]
async fn an_explicit_cancel_during_a_discard_still_cancels() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open_with(capped_config(&database, 10)).await;
    run(&mut session, "BEGIN").await;

    let token = CancellationToken::new();
    cancel_after(&token, Duration::from_millis(500));
    let started = std::time::Instant::now();
    let cancelled = run_with(
        &mut session,
        "SELECT g, pg_sleep(0.001) FROM generate_series(1, 100000) g",
        token,
    )
    .await;

    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(cancelled.outcome.status, StatementStatus::Cancelled);
    assert_eq!(cancelled.outcome.session, SessionHealth::TransactionAborted);
    run(&mut session, "ROLLBACK").await;
    let alive = run(&mut session, "SELECT 1").await;
    assert_eq!(alive.rows, vec![vec![Value::Int(1)]]);
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
    let result = run(&mut session, "SELECT pg_sleep(30)").await;

    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(cancelling.await.unwrap().unwrap(), CancelOutcome::Sent);
    let error = result.outcome.server_error.expect("57014");
    assert_eq!(error.sqlstate, "57014");
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
    let id = session.session_id();
    session.close().await;

    assert_eq!(handle.cancel().await.unwrap(), CancelOutcome::SessionClosed);
    assert_eq!(clone.cancel().await.unwrap(), CancelOutcome::SessionClosed);

    // A stranger session keeps running; the closed handle never reaches it.
    let mut stranger = driver.open_session().await.unwrap();
    let result = run(&mut stranger, "SELECT pg_sleep(0.5), 1").await;
    assert_eq!(result.outcome.status, StatementStatus::Completed);
    assert!(id.0 > 0);
}

/// Runs a statement and reports when `Columns` and the `Outcome` arrived.
async fn timed_run(
    session: &mut Box<dyn Session>,
    sql: &str,
    token: CancellationToken,
    cancel_on_columns: bool,
) -> (Duration, Duration, dbased_core::StatementOutcome) {
    let (sink, mut receiver) = RowSink::channel(8);
    let started = std::time::Instant::now();
    let watching = {
        let token = token.clone();
        async move {
            let mut columns_at = None;
            let mut outcome = None;
            while let Some(message) = receiver.recv().await {
                match message {
                    dbased_core::RowSinkMessage::Columns(_) => {
                        assert!(columns_at.is_none(), "Columns exactly once");
                        columns_at = Some(started.elapsed());
                        if cancel_on_columns {
                            token.cancel();
                        }
                    }
                    dbased_core::RowSinkMessage::Row(_) => {
                        assert!(columns_at.is_some(), "Columns before any Row");
                    }
                    dbased_core::RowSinkMessage::Outcome(found) => outcome = Some(found),
                }
            }
            (columns_at, started.elapsed(), outcome)
        }
    };
    let executing = session.execute(UserSql::new(sql), sink, token);
    let (result, (columns_at, finished_at, streamed)) = tokio::join!(executing, watching);
    let outcome = result.unwrap();
    assert_eq!(streamed.as_ref(), Some(&outcome));
    (columns_at.expect("Columns arrived"), finished_at, outcome)
}

#[tokio::test]
async fn columns_arrive_before_a_slow_statement_finishes() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;

    let (columns_at, finished_at, outcome) = timed_run(
        &mut session,
        "SELECT pg_sleep(2), 1 AS one",
        CancellationToken::new(),
        false,
    )
    .await;

    assert!(columns_at < Duration::from_millis(1200), "{columns_at:?}");
    assert!(finished_at >= Duration::from_secs(2), "{finished_at:?}");
    assert_eq!(outcome.status, StatementStatus::Completed);
    assert_eq!(outcome.rows_received, 1);
}

#[tokio::test]
async fn a_cancel_after_the_early_columns_keeps_the_session() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;
    run(&mut session, "CREATE TEMP TABLE kept (id int)").await;

    let (columns_at, finished_at, outcome) = timed_run(
        &mut session,
        "SELECT pg_sleep(30), 1",
        CancellationToken::new(),
        true,
    )
    .await;

    assert!(columns_at < Duration::from_secs(5));
    assert!(finished_at < Duration::from_secs(10));
    assert_eq!(outcome.status, StatementStatus::Cancelled);
    assert_eq!(outcome.rows_received, 0);
    assert_eq!(outcome.session, SessionHealth::Healthy);
    let after = run(&mut session, "SELECT count(*) FROM kept").await;
    assert_eq!(after.rows, vec![vec![Value::Int(0)]]);
}

#[tokio::test]
async fn a_cancel_while_the_server_parses_behind_a_lock_keeps_the_session() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;
    run(&mut session, "CREATE TABLE locked (id int)").await;

    let options = sqlx::postgres::PgConnectOptions::new_without_pgpass()
        .host("127.0.0.1")
        .port(database.server.port)
        .username("postgres")
        .password(support::PASSWORD)
        .database(&database.name)
        .ssl_mode(sqlx::postgres::PgSslMode::Disable);
    let mut admin = sqlx::PgConnection::connect_with(&options).await.unwrap();
    sqlx::raw_sql("BEGIN; LOCK TABLE locked IN ACCESS EXCLUSIVE MODE")
        .execute(&mut admin)
        .await
        .unwrap();

    let token = CancellationToken::new();
    cancel_after(&token, Duration::from_millis(500));
    let started = std::time::Instant::now();
    let (_, _, outcome) = timed_run(&mut session, "SELECT * FROM locked", token, false).await;

    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(outcome.status, StatementStatus::Cancelled);
    assert_eq!(outcome.session, SessionHealth::Healthy);
    sqlx::raw_sql("ROLLBACK").execute(&mut admin).await.unwrap();
    let after = run(&mut session, "SELECT 1").await;
    assert_eq!(after.outcome.status, StatementStatus::Completed);
}

#[tokio::test]
async fn multi_statement_input_outside_and_inside_a_transaction_still_works() {
    let database = TestDatabase::create().await;
    let (_driver, mut session) = open(&database).await;

    let outside = run(&mut session, "SELECT 1 AS a; SELECT 2").await;
    assert_eq!(outside.columns.len(), 1);
    assert_eq!(outside.rows, vec![vec![Value::Int(1)]]);
    assert!(outside.outcome.extra_statements_discarded);

    run(&mut session, "BEGIN").await;
    let inside = run(&mut session, "SELECT 1 AS a; SELECT 2").await;
    assert_eq!(inside.outcome.status, StatementStatus::Completed);
    assert_eq!(inside.outcome.session, SessionHealth::Healthy);
    let next = run(&mut session, "SELECT 3").await;
    assert_eq!(next.rows, vec![vec![Value::Int(3)]]);
}
