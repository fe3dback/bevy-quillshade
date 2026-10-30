//! Lines of the console/logs.

use project_core::DomainId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warning,
    Error,
}

/// One console/logs line: `domain` is the domain it is about, `None` for a global one (project opened, Generate
/// summary; design §6.4).
#[derive(Debug, Clone)]
pub struct Message {
    pub domain: Option<DomainId>,
    pub level: Level,
    pub text: String,
}

impl Message {
    /// A global line.
    pub fn new(level: Level, text: impl Into<String>) -> Message {
        Message { domain: None, level, text: text.into() }
    }
}
