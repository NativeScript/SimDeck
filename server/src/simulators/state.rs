use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SessionState {
    Detached,
    Attaching,
    Ready,
    Streaming,
    Failed,
    ShuttingDown,
}

impl SessionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Detached => "Detached",
            Self::Attaching => "Attaching",
            Self::Ready => "Ready",
            Self::Streaming => "Streaming",
            Self::Failed => "Failed",
            Self::ShuttingDown => "ShuttingDown",
        }
    }
}
