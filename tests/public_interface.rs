//! Uses the library from outside its crate, as an integrator does: an item that stops being public fails here.

use std::error::Error;
use std::future::Future;
use std::time::Duration;

use captchafox::{Client, Verification, VerificationUnavailable};

const SECRET: &str = "ok_11111111000000001111111100000000";
const SITEKEY: &str = "sk_11111111000000001111111100000000";

// An empty token is rejected without a request, so no network is needed.
#[tokio::test]
async fn every_public_item_is_reachable() -> Result<(), VerificationUnavailable> {
    let client = Client::new(SECRET);
    let with_timeout = Client::with_timeout(String::from(SECRET), Duration::from_secs(3));

    assert_eq!(
        describe(client.verify("").await?),
        "rejected: missing-input-response"
    );
    assert_eq!(
        describe(with_timeout.verify_with("", SITEKEY, "203.0.113.7").await?),
        "rejected: missing-input-response"
    );
    Ok(())
}

// Integrators share the client between tasks and await its futures on a multi-threaded runtime.
#[test]
fn the_client_and_its_futures_can_be_shared_between_threads() {
    fn shared<T: Send + Sync + Clone + std::fmt::Debug>(_: &T) {}
    fn sent<T: Future + Send>(_: T) {}
    fn error<T: Error + Send + Sync + 'static>() {}

    let client = Client::new(SECRET);
    shared(&client);
    sent(client.verify(""));
    sent(client.verify_with("", "", ""));
    error::<VerificationUnavailable>();
}

fn describe(verification: Verification) -> String {
    match verification {
        Verification::Passed {
            challenge_ts,
            hostname,
            ..
        } => format!(
            "passed: {} {}",
            challenge_ts.unwrap_or_default(),
            hostname.unwrap_or_default()
        ),
        Verification::Rejected { error_codes, .. } => {
            format!("rejected: {}", error_codes.join(","))
        }
    }
}
