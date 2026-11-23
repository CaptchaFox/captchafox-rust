//! Verifies tokens of the CaptchaFox widget with siteverify, with the secret key of a site.
//!
//! ```no_run
//! use captchafox::{Client, Verification};
//!
//! # async fn handle(token: &str, remote_ip: &str) {
//! let client = Client::new(std::env::var("CAPTCHAFOX_SECRET").unwrap());
//! match client.verify_with(token, "", remote_ip).await {
//!     Ok(Verification::Passed { .. }) => { /* Process the form. */ }
//!     Ok(Verification::Rejected { .. }) => { /* Reject the form. */ }
//!     Err(_unavailable) => { /* CaptchaFox gave no usable answer: decide whether the form passes. */ }
//! }
//! # }
//! ```

mod client;
mod http;
mod verification;

pub use client::Client;
pub use verification::{Verification, VerificationUnavailable};

#[cfg(test)]
mod tests;
