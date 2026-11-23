//! What the specification's cases leave out.

use std::time::{Duration, Instant};

use super::local_siteverify::{LocalSiteverify, Mode};
use crate::client::evaluate;
use crate::{Client, Verification};

const SECRET: &str = "ok_11111111000000001111111100000000";
const TIMEOUT: Duration = Duration::from_secs(2);

#[tokio::test]
async fn fields_arrive_unchanged_whatever_their_characters() {
    let server = LocalSiteverify::start(Mode::Answer(200, r#"{"success":true}"#.to_string())).await;
    let secret = "ok_a+b&c=d %ä*-._~";
    let token = "t+&=/?#% ä€";

    Client::local(secret, TIMEOUT, server.address())
        .verify_with(token, "sk_1", "2001:db8::1")
        .await
        .unwrap();

    let expected = [
        ("secret", secret),
        ("response", token),
        ("sitekey", "sk_1"),
        ("remoteIp", "2001:db8::1"),
    ]
    .map(|(name, value)| (name.to_string(), value.to_string()));
    assert_eq!(server.requests()[0].form, expected);
}

#[test]
fn a_repeated_success_cannot_be_evaluated() {
    assert!(evaluate(br#"{"success":false,"success":true}"#).is_err());
    assert!(evaluate(br#"{"success":true,"success":true}"#).is_err());
}

#[test]
fn content_after_the_object_cannot_be_evaluated() {
    assert!(evaluate(br#"{"success":true} x"#).is_err());
    assert!(evaluate(br#"{"success":true}{"success":false}"#).is_err());
}

#[test]
fn a_body_that_is_no_utf_8_cannot_be_evaluated() {
    assert!(evaluate(b"{\"success\":true,\"x\":\"\xff\"}").is_err());
}

#[test]
fn a_deeply_nested_member_is_read_without_exhausting_the_stack() {
    let body = format!(
        r#"{{"success":true,"x":{}{}}}"#,
        "[".repeat(100_000),
        "]".repeat(100_000)
    );

    assert!(matches!(
        evaluate(body.as_bytes()),
        Ok(Verification::Passed { .. })
    ));
}

#[test]
fn the_default_timeout_is_five_seconds() {
    assert_eq!(Client::new(SECRET).timeout(), Duration::from_secs(5));
}

#[tokio::test]
async fn the_timeout_ends_an_answer_that_never_completes_and_closes_the_connection() {
    let server = LocalSiteverify::start(Mode::Trickle(200, r#"{"success":true"#.to_string())).await;
    let client = Client::local(SECRET, Duration::from_secs(1), server.address());
    let start = Instant::now();

    // The bound fails the test instead of letting it hang when the timeout does not take effect.
    let outcome = tokio::time::timeout(Duration::from_secs(10), client.verify("token")).await;
    assert!(matches!(outcome, Ok(Err(_))), "{outcome:?}");
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(3),
        "the verification took {elapsed:?}"
    );

    // The server notices the closed connection when it writes its next space.
    let limit = Instant::now() + Duration::from_secs(5);
    while server.broken_at().is_none() && Instant::now() < limit {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(server.broken_at().is_some(), "the connection is still open");
}

#[test]
fn the_secret_key_is_not_in_the_debug_form() {
    let debug = format!("{:?}", Client::new(SECRET));

    assert!(!debug.contains(SECRET), "{debug}");
}

#[test]
#[should_panic(expected = "the secret key is empty")]
fn an_empty_secret_key_is_refused() {
    Client::new("");
}

#[test]
#[should_panic(expected = "the timeout is zero")]
fn a_zero_timeout_is_refused() {
    Client::with_timeout(SECRET, Duration::ZERO);
}
