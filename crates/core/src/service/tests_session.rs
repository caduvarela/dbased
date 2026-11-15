#![allow(clippy::unwrap_used, clippy::expect_used)]

//! A scripted `Session`: the statement text picks the behavior, like the fake Core of 11.4.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use dbased_commands::{ColumnMeta, Value};
use tokio_util::sync::CancellationToken;

use crate::{
    CancelHandle, Canceller, CloseReason, CoreError, DatabaseErrorInfo, RowSink, RowSinkMessage,
    Session, SessionHealth, SessionId, StatementOutcome, StatementStatus, UserSql,
};

#[derive(Default)]
pub(super) struct SessionLog {
    pub(super) executed: Mutex<Vec<String>>,
    pub(super) order: Mutex<Vec<&'static str>>,
    pub(super) opens: AtomicUsize,
    pub(super) exports: AtomicUsize,
    pub(super) closes: AtomicUsize,
    pub(super) started: AtomicUsize,
    pub(super) slow_rows_sent: AtomicBool,
}

pub(super) struct ScriptedSession {
    pub(super) log: Arc<SessionLog>,
}

struct NoCancel;

#[async_trait]
impl Canceller for NoCancel {
    async fn cancel_backend(&self, _backend: SessionId) -> Result<(), CoreError> {
        Ok(())
    }
}

fn outcome(status: StatementStatus, rows: u64) -> StatementOutcome {
    StatementOutcome {
        status,
        rows_received: rows,
        rows_affected: None,
        partial: false,
        server_error: None,
        cancel_error: None,
        extra_statements_discarded: false,
        session: SessionHealth::Healthy,
    }
}

fn column() -> Vec<ColumnMeta> {
    vec![ColumnMeta {
        name: "n".into(),
        type_name: "int8".into(),
        nullable: Some(false),
    }]
}

async fn stream_rows(sink: &RowSink, count: u64) {
    sink.send(RowSinkMessage::Columns(column())).await.unwrap();
    for number in 0..count {
        let row = vec![Value::Int(i64::try_from(number).unwrap())];
        sink.send(RowSinkMessage::Row(row)).await.unwrap();
    }
}

async fn finish(sink: &RowSink, result: StatementOutcome) -> Result<StatementOutcome, CoreError> {
    sink.send(RowSinkMessage::Outcome(result.clone()))
        .await
        .unwrap();
    Ok(result)
}

#[async_trait]
impl Session for ScriptedSession {
    fn session_id(&self) -> SessionId {
        SessionId(1)
    }

    fn cancel_handle(&self) -> CancelHandle {
        CancelHandle::new(SessionId(1), Arc::new(NoCancel))
    }

    async fn execute(
        &mut self,
        sql: UserSql,
        sink: RowSink,
        token: CancellationToken,
    ) -> Result<StatementOutcome, CoreError> {
        let text = sql.as_str().to_owned();
        self.log.executed.lock().unwrap().push(text.clone());
        self.log.started.fetch_add(1, Ordering::SeqCst);
        let mut words = text.split_whitespace();
        let command = words.next().unwrap_or("");
        let argument: u64 = words.next().and_then(|word| word.parse().ok()).unwrap_or(1);

        let result = match command {
            "rows" => {
                stream_rows(&sink, argument).await;
                outcome(StatementStatus::Completed, argument)
            }
            "truncate" => {
                stream_rows(&sink, argument).await;
                let mut truncated = outcome(StatementStatus::Truncated, argument);
                truncated.partial = true;
                truncated
            }
            "affect" => {
                sink.send(RowSinkMessage::Columns(Vec::new()))
                    .await
                    .unwrap();
                let mut done = outcome(StatementStatus::Completed, 0);
                done.rows_affected = Some(argument);
                done
            }
            "fail" => {
                sink.send(RowSinkMessage::Columns(Vec::new()))
                    .await
                    .unwrap();
                let mut failed = outcome(StatementStatus::Failed, 0);
                failed.server_error = Some(DatabaseErrorInfo {
                    sqlstate: "42601".into(),
                    message: "syntax error".into(),
                    detail: None,
                    hint: None,
                    position: Some(argument.try_into().unwrap()),
                });
                failed
            }
            "sleep" => {
                sink.send(RowSinkMessage::Columns(column())).await.unwrap();
                token.cancelled().await;
                let mut cancelled = outcome(StatementStatus::Cancelled, 0);
                cancelled.partial = true;
                cancelled
            }
            "slow" => {
                sink.send(RowSinkMessage::Columns(column())).await.unwrap();
                for number in 0..argument {
                    let row = vec![Value::Int(i64::try_from(number).unwrap())];
                    sink.send(RowSinkMessage::Row(row)).await.unwrap();
                    tokio::time::sleep(Duration::from_millis(120)).await;
                }
                self.log.slow_rows_sent.store(true, Ordering::SeqCst);
                outcome(StatementStatus::Completed, argument)
            }
            "paced" => {
                sink.send(RowSinkMessage::Columns(column())).await.unwrap();
                let mut sent = 0;
                for number in 0..argument {
                    if token.is_cancelled() {
                        let mut cancelled = outcome(StatementStatus::Cancelled, sent);
                        cancelled.partial = true;
                        return finish(&sink, cancelled).await;
                    }
                    let row = vec![Value::Int(i64::try_from(number).unwrap())];
                    sink.send(RowSinkMessage::Row(row)).await.unwrap();
                    sent += 1;
                    tokio::time::sleep(Duration::from_millis(30)).await;
                }
                outcome(StatementStatus::Completed, sent)
            }
            "mixed" => {
                let columns = ["id", "note"].map(|name| ColumnMeta {
                    name: name.into(),
                    type_name: "text".into(),
                    nullable: None,
                });
                sink.send(RowSinkMessage::Columns(columns.to_vec()))
                    .await
                    .unwrap();
                let rows = [
                    vec![Value::Int(1), Value::Text("a,b".into())],
                    vec![Value::Int(2), Value::Null],
                    vec![Value::Int(3), Value::Text("line\nbreak \"q\"".into())],
                    vec![Value::Int(4), Value::Text(String::new())],
                ];
                for row in rows {
                    sink.send(RowSinkMessage::Row(row)).await.unwrap();
                }
                outcome(StatementStatus::Completed, 4)
            }
            "failafter" => {
                stream_rows(&sink, argument).await;
                let mut failed = outcome(StatementStatus::Failed, argument);
                failed.server_error = Some(DatabaseErrorInfo {
                    sqlstate: "22012".into(),
                    message: "division by zero".into(),
                    detail: None,
                    hint: None,
                    position: None,
                });
                failed
            }
            "dead" => return Err(CoreError::SessionClosed(CloseReason::ConnectionError)),
            "deadrows" => {
                stream_rows(&sink, 2).await;
                return Err(CoreError::SessionClosed(CloseReason::ConnectionError));
            }
            "closed" => {
                stream_rows(&sink, 1).await;
                let mut closed = outcome(StatementStatus::Completed, 1);
                closed.session = SessionHealth::Closed(CloseReason::DrainTimeout);
                closed
            }
            "aborted" => {
                sink.send(RowSinkMessage::Columns(Vec::new()))
                    .await
                    .unwrap();
                let mut failed = outcome(StatementStatus::Failed, 0);
                failed.session = SessionHealth::TransactionAborted;
                failed
            }
            "extra" => {
                stream_rows(&sink, 1).await;
                let mut extra = outcome(StatementStatus::Completed, 1);
                extra.extra_statements_discarded = true;
                extra
            }
            _ => {
                stream_rows(&sink, 1).await;
                outcome(StatementStatus::Completed, 1)
            }
        };
        finish(&sink, result).await
    }

    async fn health_check(&mut self) -> Result<SessionHealth, CoreError> {
        Ok(SessionHealth::Healthy)
    }

    async fn close(self: Box<Self>) {
        self.log.order.lock().unwrap().push("session_close");
        self.log.closes.fetch_add(1, Ordering::SeqCst);
    }
}
