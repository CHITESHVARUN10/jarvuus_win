#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileHit {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub modified: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActionOutcome {
    pub ok: bool,
    pub message: String,
    pub files: Vec<FileHit>,
}

impl ActionOutcome {
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: message.into(),
            files: Vec::new(),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: message.into(),
            files: Vec::new(),
        }
    }

    pub fn with_files(message: impl Into<String>, files: Vec<FileHit>) -> Self {
        Self {
            ok: true,
            message: message.into(),
            files,
        }
    }
}
