//! Single error type for all profile identity, selection and table resolution.
//!
//! The category records which profile boundary refused the input; the message
//! preserves the observed detail without exposing one public variant per site.

/// Error raised by profile identity, selection, registry or table resolution.
///
/// This enum carries no `#[non_exhaustive]`, on purpose: a caller may match it
/// exhaustively, so adding a variant is a deliberate breaking change — the
/// compiler must tell every caller that a new failure mode exists. Removing a
/// variant that no longer has a producing site follows the same rule and is
/// equally deliberate (docs/reference/standards.md, Errors).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileError {
    /// Profile identity or carried setup is inconsistent.
    Identity { message: String },
    /// No installed profile can be selected for the requested stream.
    Selection { message: String },
    /// A frozen codebook or mode table cannot answer the request.
    Tables { message: String },
    /// An optional quality curve is malformed or cannot answer the request.
    Quality { message: String },
    /// Downstream algorithm errors retain their source chain.
    /// wem-analysis structural validation failed.
    Analysis(wem_analysis::config::AnalysisError),
    /// wem-vorbis codebook construction/parse failed.
    Codebook(wem_vorbis::codebook::CodebookError),
    /// wem-vorbis bitstream access failed during setup parsing.
    Bit(wem_vorbis::bitio::BitError),
}

impl ProfileError {
    pub fn identity(message: impl Into<String>) -> Self {
        Self::Identity {
            message: message.into(),
        }
    }

    pub fn selection(message: impl Into<String>) -> Self {
        Self::Selection {
            message: message.into(),
        }
    }

    pub fn tables(message: impl Into<String>) -> Self {
        Self::Tables {
            message: message.into(),
        }
    }

    pub fn quality(message: impl Into<String>) -> Self {
        Self::Quality {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Identity { message }
            | Self::Selection { message }
            | Self::Tables { message }
            | Self::Quality { message } => f.write_str(message),
            Self::Analysis(e) => write!(f, "analysis error: {e}"),
            Self::Codebook(e) => write!(f, "codebook error: {e}"),
            Self::Bit(e) => write!(f, "bitstream error: {e}"),
        }
    }
}

impl std::error::Error for ProfileError {
    /// The wrapped analysis/codebook/bitstream failure; a data error with an
    /// inline message has no nested cause of its own.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Analysis(e) => Some(e),
            Self::Codebook(e) => Some(e),
            Self::Bit(e) => Some(e),
            _ => None,
        }
    }
}
