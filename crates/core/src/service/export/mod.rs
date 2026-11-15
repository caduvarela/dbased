//! CSV export (docs/PROJECT.md 3.3): a fresh run of one statement on its own dedicated session,
//! streamed to disk without ever holding the result. Rows are encoded into small chunks and a
//! blocking writer task appends them to a temporary file next to the target; the file only
//! takes the target's name once the statement completed, so a failed or cancelled export never
//! leaves a partial file behind.

mod encoder;
#[cfg(test)]
mod encoder_tests;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_channel::Sender;
use dbased_commands::{ColumnMeta, CsvOptions, Event, ExportStatus, RequestId, Value};
use dbased_sql::{Dialect, split_statements};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use self::encoder::CsvEncoder;
use super::error_info::error_info;
use crate::{
    CoreError, Driver, RowSink, RowSinkMessage, StatementOutcome, StatementStatus, UserSql,
};

const SINK_CAPACITY: usize = 1_024;
const CHUNK_BYTES: usize = 64 * 1024;
/// Chunks in flight between the encoder and the blocking writer; this bounds memory.
const CHUNKS_IN_FLIGHT: usize = 4;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// One accepted `ExportCsv` request.
pub(super) struct ExportJob {
    pub(super) request: RequestId,
    pub(super) sql: String,
    pub(super) path: PathBuf,
    pub(super) options: CsvOptions,
    pub(super) dialect: Dialect,
    /// Cancelled by `CancelExport`, a disconnect or shutdown.
    pub(super) token: CancellationToken,
    pub(super) events: Sender<Event>,
}

/// Runs the export and returns its single terminal event: `ExportFinished` or `RequestFailed`.
pub(super) async fn run_export(driver: Arc<dyn Driver>, job: ExportJob) -> Event {
    let request = job.request;
    match export(driver.as_ref(), &job).await {
        Ok(finished) => Event::ExportFinished {
            request,
            status: finished.status,
            rows_written: finished.rows_written,
            bytes_written: finished.bytes_written,
        },
        Err(error) => Event::RequestFailed {
            request,
            error: error_info(&error),
        },
    }
}

struct Finished {
    status: ExportStatus,
    rows_written: u64,
    bytes_written: u64,
}

async fn export(driver: &dyn Driver, job: &ExportJob) -> Result<Finished, CoreError> {
    let statement = single_statement(&job.sql, job.dialect)?;
    let encoder = CsvEncoder::new(&job.options)?;
    let temporary = temporary_path(&job.path, job.request)?;

    // @NOTE: the file is created before the query starts, so a bad path fails fast.
    let file = blocking(storage_context("create", {
        let temporary = temporary.clone();
        move || std::fs::File::create(temporary)
    }))
    .await?;
    let writer = FileWriter::start(file);

    let result = stream_to_file(driver, job, statement, encoder, writer).await;
    let finished = match result {
        Ok(finished) => finished,
        Err(error) => {
            remove_file(&temporary).await;
            return Err(error);
        }
    };
    if finished.status == ExportStatus::Cancelled {
        remove_file(&temporary).await;
        return Ok(finished);
    }
    let renamed = blocking(storage_context("finalize", {
        let (temporary, target) = (temporary.clone(), job.path.clone());
        move || std::fs::rename(temporary, target)
    }))
    .await;
    if let Err(error) = renamed {
        remove_file(&temporary).await;
        return Err(error);
    }
    Ok(finished)
}

/// The only place a user-typed export statement becomes `UserSql`: exactly one statement.
fn single_statement(sql: &str, dialect: Dialect) -> Result<UserSql, CoreError> {
    let statements = split_statements(sql, dialect);
    let mut spans = statements.iter().map(|statement| statement.span);
    let (Some(span), None) = (spans.next(), spans.next()) else {
        return Err(if statements.is_empty() {
            CoreError::EmptyScript
        } else {
            CoreError::Unsupported("an export runs exactly one statement")
        });
    };
    let text = sql
        .get(span.start..span.end)
        .ok_or_else(|| CoreError::Internal("the statement is outside the script".into()))?;
    Ok(UserSql::new(text))
}

fn temporary_path(target: &Path, request: RequestId) -> Result<PathBuf, CoreError> {
    let name = target
        .file_name()
        .ok_or_else(|| CoreError::Configuration("the export path has no file name".into()))?;
    let mut temporary_name = std::ffi::OsString::from(".");
    temporary_name.push(name);
    temporary_name.push(format!(".{}.part", request.0));
    Ok(target.with_file_name(temporary_name))
}

async fn stream_to_file(
    driver: &dyn Driver,
    job: &ExportJob,
    statement: UserSql,
    encoder: CsvEncoder,
    writer: FileWriter,
) -> Result<Finished, CoreError> {
    let mut session = match driver.open_export_session().await {
        Ok(session) => session,
        Err(error) => {
            writer.abandon().await;
            return Err(error);
        }
    };
    // @NOTE: a write failure cancels the statement through this child; the user's token stays
    // the only thing that means "cancelled by the user".
    let stop = job.token.child_token();
    let (sink, mut receiver) = RowSink::channel(SINK_CAPACITY);
    let mut streamer = Streamer::new(job, encoder, writer, stop.clone());
    // @NOTE: join, never select: the execute future runs to its terminal result (3.2).
    let (executed, ()) = tokio::join!(
        session.execute(statement, sink, stop),
        streamer.run(&mut receiver),
    );
    // @NOTE: a dedicated session is never reused, and callers always close (3.2.4).
    session.close().await;

    let streamed = streamer.finish().await;
    let outcome = executed?;
    let counts = streamed?;
    finished_from(outcome, counts)
}

fn finished_from(outcome: StatementOutcome, counts: Counts) -> Result<Finished, CoreError> {
    if outcome.extra_statements_discarded {
        return Err(CoreError::Internal(
            "an export statement carried more than one statement".into(),
        ));
    }
    let status = match outcome.status {
        StatementStatus::Completed => ExportStatus::Completed,
        StatementStatus::Cancelled => ExportStatus::Cancelled,
        StatementStatus::Failed => {
            return Err(match outcome.server_error {
                Some(info) => CoreError::Database(info),
                None => CoreError::Protocol("the statement failed without an error".into()),
            });
        }
        StatementStatus::Truncated => {
            return Err(CoreError::Internal(
                "an export session applied a result cap".into(),
            ));
        }
    };
    if status == ExportStatus::Completed && !counts.has_columns {
        return Err(CoreError::Unsupported("the statement returns no rows"));
    }
    Ok(Finished {
        status,
        rows_written: counts.rows,
        bytes_written: counts.bytes,
    })
}

struct Counts {
    rows: u64,
    bytes: u64,
    has_columns: bool,
}

/// Encodes sink rows into chunks for the writer and reports progress.
struct Streamer {
    request: RequestId,
    events: Sender<Event>,
    encoder: CsvEncoder,
    options: CsvOptions,
    writer: FileWriter,
    stop: CancellationToken,
    pending: Vec<u8>,
    rows: u64,
    bytes: u64,
    has_columns: bool,
    write_failed: bool,
    last_progress: Option<Instant>,
}

impl Streamer {
    fn new(
        job: &ExportJob,
        encoder: CsvEncoder,
        writer: FileWriter,
        stop: CancellationToken,
    ) -> Self {
        Self {
            request: job.request,
            events: job.events.clone(),
            encoder,
            options: job.options,
            writer,
            stop,
            pending: Vec::with_capacity(CHUNK_BYTES * 2),
            rows: 0,
            bytes: 0,
            has_columns: false,
            write_failed: false,
            last_progress: None,
        }
    }

    /// Reads until the outcome. After a write failure it keeps reading and discarding, so the
    /// driver is never blocked on a full sink while it drains the cancelled statement.
    async fn run(&mut self, receiver: &mut mpsc::Receiver<RowSinkMessage>) {
        while let Some(message) = receiver.recv().await {
            match message {
                RowSinkMessage::Columns(columns) => self.start(&columns),
                RowSinkMessage::Row(row) => self.append(&row).await,
                RowSinkMessage::Outcome(_) => return,
            }
        }
    }

    fn start(&mut self, columns: &[ColumnMeta]) {
        if columns.is_empty() || self.has_columns {
            return;
        }
        self.has_columns = true;
        if self.options.bom {
            self.encoder.write_byte_order_mark(&mut self.pending);
        }
        if self.options.header {
            self.encoder.write_header(columns, &mut self.pending);
        }
    }

    async fn append(&mut self, row: &[Value]) {
        if self.write_failed {
            return;
        }
        self.encoder.write_row(row, &mut self.pending);
        self.rows += 1;
        if self.pending.len() >= CHUNK_BYTES {
            self.flush().await;
        }
        self.report_progress();
    }

    async fn flush(&mut self) {
        if self.pending.is_empty() || self.write_failed {
            return;
        }
        let chunk = std::mem::replace(&mut self.pending, Vec::with_capacity(CHUNK_BYTES * 2));
        let length = chunk.len() as u64;
        if self.writer.send(chunk).await {
            self.bytes += length;
            return;
        }
        // @NOTE: the writer stopped on an I/O error; its error surfaces in `finish`.
        self.write_failed = true;
        self.stop.cancel();
    }

    /// First rows at once, then at most every 100 ms; dropped when the event channel is full.
    fn report_progress(&mut self) {
        let now = Instant::now();
        let due = self
            .last_progress
            .is_none_or(|last| now.duration_since(last) >= PROGRESS_INTERVAL);
        if !due {
            return;
        }
        self.last_progress = Some(now);
        self.events
            .try_send(Event::ExportProgress {
                request: self.request,
                rows_written: self.rows,
                bytes_written: self.bytes,
            })
            .ok();
    }

    /// Flushes the tail, waits for the writer and returns what was written.
    async fn finish(mut self) -> Result<Counts, CoreError> {
        self.flush().await;
        self.writer.finish().await?;
        Ok(Counts {
            rows: self.rows,
            bytes: self.bytes,
            has_columns: self.has_columns,
        })
    }
}

/// A blocking task that appends chunks to the file, so disk latency never stalls the runtime.
struct FileWriter {
    chunks: Option<mpsc::Sender<Vec<u8>>>,
    task: JoinHandle<std::io::Result<()>>,
}

impl FileWriter {
    fn start(mut file: std::fs::File) -> Self {
        let (chunks, mut incoming) = mpsc::channel::<Vec<u8>>(CHUNKS_IN_FLIGHT);
        let task = tokio::task::spawn_blocking(move || {
            while let Some(chunk) = incoming.blocking_recv() {
                file.write_all(&chunk)?;
            }
            file.flush()?;
            file.sync_all()
        });
        Self {
            chunks: Some(chunks),
            task,
        }
    }

    /// `false` once the writer task has stopped, which only happens after an I/O error.
    async fn send(&self, chunk: Vec<u8>) -> bool {
        match &self.chunks {
            Some(chunks) => chunks.send(chunk).await.is_ok(),
            None => false,
        }
    }

    async fn finish(mut self) -> Result<(), CoreError> {
        self.chunks.take();
        match self.task.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(CoreError::Storage(format!(
                "could not write the export file: {error}"
            ))),
            Err(error) => Err(CoreError::Internal(format!(
                "the export writer failed: {error}"
            ))),
        }
    }

    async fn abandon(mut self) {
        self.chunks.take();
        self.task.await.ok();
    }
}

fn storage_context<T: Send + 'static>(
    action: &'static str,
    work: impl FnOnce() -> std::io::Result<T> + Send + 'static,
) -> impl FnOnce() -> Result<T, CoreError> + Send + 'static {
    move || {
        work().map_err(|error| {
            CoreError::Storage(format!("could not {action} the export file: {error}"))
        })
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, CoreError> + Send + 'static,
) -> Result<T, CoreError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| CoreError::Internal(format!("blocking task failed: {error}")))?
}

/// Best effort: the partial file is garbage either way.
async fn remove_file(path: &Path) {
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || std::fs::remove_file(path))
        .await
        .ok();
}
