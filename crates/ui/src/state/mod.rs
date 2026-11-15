//! View models of the UI, free of GPUI so they can be unit tested (docs/PROJECT.md 6.2, 7.1).

mod connection_flow;
mod connection_form;
mod connections;
mod explorer;
mod format;
mod ids;
mod results;
mod tabs;
mod ui_state;
mod window_cache;

pub use connection_flow::{SaveStatus, TestStatus};
pub use connection_form::{ConnectionForm, FormErrors, tls_label, tls_warning};
pub use connections::kind_label;
pub use explorer::{ExplorerNode, NodeKey, NodeKind};
pub use format::is_numeric_type;
pub use results::ResultModel;
pub use tabs::TabStatus;
pub use ui_state::{SendFailure, UiState};
pub use window_cache::{CellLookup, FetchRequest, WindowCache};
