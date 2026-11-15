use dbased_commands::{RequestId, TabId};

/// Hands out the ids the UI owns (docs/PROJECT.md 11.1): one process-wide counter each.
#[derive(Debug, Default)]
pub struct IdAllocator {
    last_tab: u64,
    last_request: u64,
}

impl IdAllocator {
    pub fn tab(&mut self) -> TabId {
        self.last_tab += 1;
        TabId(self.last_tab)
    }

    pub fn request(&mut self) -> RequestId {
        self.last_request += 1;
        RequestId(self.last_request)
    }
}
