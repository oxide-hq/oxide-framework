use std::io;
use std::path::PathBuf;

/// Everything that can stop an `oxide` command.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// Bad input: an invalid name, conflicting flags, a file that already exists, ...
    #[error("{0}")]
    Invalid(String),

    /// Reading or writing a file failed.
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// A terminal prompt failed.
    #[error("prompt failed: {0}")]
    Prompt(#[source] io::Error),

    /// The user pressed Ctrl-C or Esc in the wizard.
    #[error("cancelled")]
    Cancelled,
}

impl CliError {
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }

    pub(crate) fn io(path: impl Into<PathBuf>) -> impl FnOnce(io::Error) -> Self {
        let path = path.into();
        move |source| Self::Io { path, source }
    }

    pub(crate) fn prompt(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::Interrupted {
            Self::Cancelled
        } else {
            Self::Prompt(error)
        }
    }
}
