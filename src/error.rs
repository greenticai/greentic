use std::path::StripPrefixError;

use thiserror::Error;

pub type GtcResult<T> = Result<T, GtcError>;

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
    use super::GtcError;

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
}
