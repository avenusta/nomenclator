//! Reads YAML and JSON documents in the one dialect that every document of the toolkit
//! uses: overlays, language packs, message catalogues, caller inputs, profiles and corpus
//! cases.

use std::fmt;

use serde::de::DeserializeOwned;

/// A document that could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentError {
    /// The 1-based line of the problem, when the parser knows it.
    pub line: Option<usize>,
    /// The parser's description of the problem.
    pub message: String,
}

impl fmt::Display for DocumentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(formatter, "line {line}: {}", self.message),
            None => formatter.write_str(&self.message),
        }
    }
}

impl std::error::Error for DocumentError {}

/// Reads a YAML or JSON document into `T`. Booleans are only `true` and `false`, as in
/// YAML 1.2, so `off`, `no` and `y` stay strings. A mapping with a repeated key is
/// rejected.
///
/// Fails with the line and the parser's description if `text` is not a valid document
/// of `T`.
pub fn parse_document<T: DeserializeOwned>(text: &str) -> Result<T, DocumentError> {
    let options = serde_saphyr::options! { strict_booleans: true };
    serde_saphyr::from_str_with_options(text, options).map_err(|error| DocumentError {
        line: error
            .location()
            .and_then(|location| usize::try_from(location.line()).ok()),
        message: error.without_snippet().to_string(),
    })
}
