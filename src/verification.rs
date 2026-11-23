use std::error::Error;
use std::fmt;

/// The outcome of a token that siteverify evaluated: either [`Passed`](Verification::Passed) or
/// [`Rejected`](Verification::Rejected). A token whose verification cannot be evaluated has no `Verification`;
/// [`Client::verify`](crate::Client::verify) returns [`VerificationUnavailable`] instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verification {
    /// siteverify answered `"success": true`.
    // Non-exhaustive, so that a field siteverify adds later can be passed on within a major version.
    #[non_exhaustive]
    Passed {
        /// The time of the challenge as siteverify sent it, an ISO 8601 timestamp, if siteverify sent one.
        challenge_ts: Option<String>,
        /// The hostname of the site where the challenge was solved, if siteverify sent one.
        hostname: Option<String>,
    },
    /// siteverify answered `"success": false`, or the token was empty.
    #[non_exhaustive]
    Rejected {
        /// The error codes of siteverify, such as `timeout-or-duplicate`, possibly none.
        error_codes: Vec<String>,
    },
}

/// siteverify could not be evaluated: it was not reached, did not answer in time, answered with an HTTP status other
/// than 200, or with a body that is no JSON object with a boolean `success`. Whether the protected request passes then
/// is the caller's decision. Its message never contains the secret key.
#[derive(Debug)]
pub struct VerificationUnavailable {
    message: String,
    source: Option<std::io::Error>,
}

impl VerificationUnavailable {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        VerificationUnavailable {
            message: message.into(),
            source: None,
        }
    }

    pub(crate) fn caused_by(message: impl Into<String>, source: std::io::Error) -> Self {
        VerificationUnavailable {
            message: message.into(),
            source: Some(source),
        }
    }
}

impl fmt::Display for VerificationUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for VerificationUnavailable {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.source {
            Some(source) => Some(source),
            None => None,
        }
    }
}
