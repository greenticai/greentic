use std::error::Error as StdError;
use std::path::StripPrefixError;

use thiserror::Error;

pub type GtcResult<T> = Result<T, GtcError>;

/// Render an error and its full `source` chain as a single line.
///
/// Most errors reaching [`GtcError::Message`] are flattened with `{err}`, which
/// prints only the outermost message. For a failed registry pull that reads
/// `error sending request for url (...)` and drops the cause that actually
/// explains it -- connection reset, TLS failure, DNS failure -- leaving the
/// operator nothing to act on. Walking the chain keeps that detail.
///
/// `greentic-distributor-client` carries an identical helper next to the retry
/// policy that produces these errors. It is duplicated rather than imported
/// because this crate depends on that one through a published version range,
/// so sharing it would gate every `gtc` change on a distributor-client release.
pub fn error_chain(error: &dyn StdError) -> String {
    let mut rendered = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        // `#[error(transparent)]` wrappers repeat their source verbatim; the
        // duplicate would only make the line harder to read.
        if !rendered.ends_with(&text) {
            rendered.push_str(": ");
            rendered.push_str(&text);
        }
        source = cause.source();
    }
    rendered
}

#[derive(Debug, Error)]
pub enum GtcError {
    #[error("{0}")]
    Message(String),

    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },

    #[error("{context}: {source}")]
    Json {
        context: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("{context}: {source}")]
    Path {
        context: String,
        #[source]
        source: StripPrefixError,
    },

    #[error("{context}: {details}")]
    InvalidData { context: String, details: String },
}

impl GtcError {
    pub fn contains(&self, needle: &str) -> bool {
        self.to_string().contains(needle)
    }

    pub fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }

    pub fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }

    pub fn json(context: impl Into<String>, source: serde_json::Error) -> Self {
        Self::Json {
            context: context.into(),
            source,
        }
    }

    pub fn path(context: impl Into<String>, source: StripPrefixError) -> Self {
        Self::Path {
            context: context.into(),
            source,
        }
    }

    pub fn invalid_data(context: impl Into<String>, details: impl Into<String>) -> Self {
        Self::InvalidData {
            context: context.into(),
            details: details.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use super::{GtcError, error_chain};

    #[test]
    fn message_variant_preserves_text() {
        let err = GtcError::message("plain error");
        assert_eq!(err.to_string(), "plain error");
    }

    #[test]
    fn invalid_data_variant_includes_context() {
        let err = GtcError::invalid_data("index.json", "root must be an object");
        assert_eq!(err.to_string(), "index.json: root must be an object");
    }

    #[test]
    fn error_chain_appends_every_cause() {
        #[derive(Debug, thiserror::Error)]
        #[error("oci pack error")]
        struct Middle {
            #[source]
            source: std::io::Error,
        }

        #[derive(Debug, thiserror::Error)]
        #[error("failed to pull `ghcr.io/example:1.0.0`")]
        struct Outer {
            #[source]
            source: Middle,
        }

        let rendered = error_chain(&Outer {
            source: Middle {
                source: std::io::Error::new(ErrorKind::ConnectionReset, "connection reset by peer"),
            },
        });
        assert_eq!(
            rendered,
            "failed to pull `ghcr.io/example:1.0.0`: oci pack error: connection reset by peer"
        );
    }

    #[test]
    fn error_chain_does_not_repeat_transparent_wrappers() {
        #[derive(Debug, thiserror::Error)]
        #[error(transparent)]
        struct Transparent {
            #[from]
            source: std::io::Error,
        }

        let rendered = error_chain(&Transparent {
            source: std::io::Error::new(ErrorKind::ConnectionReset, "connection reset by peer"),
        });
        assert_eq!(rendered, "connection reset by peer");
    }

    #[test]
    fn error_chain_of_a_sourceless_error_is_just_its_message() {
        let err = GtcError::message("plain error");
        assert_eq!(error_chain(&err), "plain error");
    }
}
