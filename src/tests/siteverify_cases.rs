//! Runs every case of the siteverify specification against a local siteverify.

use std::error::Error;
use std::time::Duration;

use serde_json::Value;

use super::local_siteverify::{refusing_address, LocalSiteverify, Mode};
use crate::client::{HOST, PATH};
use crate::{Client, Verification, VerificationUnavailable};

const TIMEOUT: Duration = Duration::from_secs(5);
// Short, so that the cases of a siteverify that stops answering end quickly.
const SILENCE_TIMEOUT: Duration = Duration::from_secs(1);
// Fails a case whose timeout does not take effect instead of letting it hang.
const CASE_BOUND: Duration = Duration::from_secs(10);

fn specification() -> Value {
    serde_json::from_str(include_str!("__fixtures__/siteverify-cases.json")).unwrap()
}

#[tokio::test]
async fn every_case() {
    let specification = specification();
    let cases = specification["cases"].as_array().unwrap();
    assert!(!cases.is_empty());

    for case in cases {
        let name = case["name"].as_str().unwrap();
        if tokio::time::timeout(CASE_BOUND, run(case)).await.is_err() {
            panic!("{name}: did not end within {CASE_BOUND:?}");
        }
    }
}

#[test]
fn the_endpoint_is_the_one_of_the_specification() {
    assert_eq!(format!("https://{HOST}{PATH}"), specification()["endpoint"]);
}

async fn run(case: &Value) {
    let name = case["name"].as_str().unwrap();
    let input = &case["input"];
    let answer = &case["answer"];
    let secret = input["secret"].as_str().unwrap();

    let mut timeout = TIMEOUT;
    let server = if answer.is_null() {
        Some(LocalSiteverify::start(Mode::Answer(500, String::new())).await)
    } else if answer.get("error").is_none() {
        Some(LocalSiteverify::start(Mode::Answer(status(answer), text(answer, "body"))).await)
    } else {
        match answer["error"].as_str().unwrap() {
            "timeout" => {
                timeout = SILENCE_TIMEOUT;
                Some(LocalSiteverify::start(Mode::Silent).await)
            }
            "stall" => {
                timeout = SILENCE_TIMEOUT;
                Some(
                    LocalSiteverify::start(Mode::Stall(status(answer), text(answer, "body"))).await,
                )
            }
            "connection" => None,
            error => panic!("{name}: unknown answer error {error}"),
        }
    };
    let address = match &server {
        Some(server) => server.address(),
        None => refusing_address().await,
    };

    let client = Client::local(secret, timeout, address);
    let outcome = client
        .verify_with(
            input["response"].as_str().unwrap(),
            input["sitekey"].as_str().unwrap_or(""),
            input["remoteIp"].as_str().unwrap_or(""),
        )
        .await;

    expect_outcome(name, &case["expect"], secret, outcome);
    expect_request(name, &case["request"], server.as_ref());
}

fn expect_outcome(
    name: &str,
    expect: &Value,
    secret: &str,
    outcome: Result<Verification, VerificationUnavailable>,
) {
    match expect["outcome"].as_str().unwrap() {
        "passed" => match outcome {
            Ok(Verification::Passed {
                challenge_ts,
                hostname,
                ..
            }) => {
                assert_eq!(
                    challenge_ts.as_deref(),
                    expect["challengeTs"].as_str(),
                    "{name}"
                );
                assert_eq!(hostname.as_deref(), expect["hostname"].as_str(), "{name}");
            }
            other => panic!("{name}: expected passed, got {other:?}"),
        },
        "rejected" => match outcome {
            Ok(Verification::Rejected { error_codes, .. }) => {
                let expected: Vec<&str> = expect["errorCodes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|code| code.as_str().unwrap())
                    .collect();
                assert_eq!(error_codes, expected, "{name}");
            }
            other => panic!("{name}: expected rejected, got {other:?}"),
        },
        "unavailable" => match outcome {
            Err(unavailable) => {
                assert!(
                    !format!("{unavailable:?}").contains(secret),
                    "{name}: the secret key is in {unavailable:?}"
                );
                let mut cause: Option<&dyn Error> = Some(&unavailable);
                while let Some(error) = cause {
                    assert!(
                        !error.to_string().contains(secret),
                        "{name}: the secret key is in {error}"
                    );
                    cause = error.source();
                }
            }
            other => panic!("{name}: expected unavailable, got {other:?}"),
        },
        outcome => panic!("{name}: unknown outcome {outcome}"),
    }
}

fn expect_request(name: &str, request: &Value, server: Option<&LocalSiteverify>) {
    let Some(server) = server else {
        assert!(
            request.is_null(),
            "{name}: a request is expected, but no server ran"
        );
        return;
    };
    let received = server.requests();

    if request.is_null() {
        assert!(received.is_empty(), "{name}: a request arrived");
        return;
    }

    assert_eq!(received.len(), 1, "{name}: one request arrives");
    assert_eq!(received[0].method, "POST", "{name}");
    assert_eq!(received[0].target, PATH, "{name}");
    assert_eq!(
        received[0].content_type.as_deref(),
        Some("application/x-www-form-urlencoded"),
        "{name}"
    );

    let mut fields = received[0].form.clone();
    fields.sort();
    let mut expected: Vec<(String, String)> = request
        .as_object()
        .unwrap()
        .iter()
        .map(|(field, value)| (field.clone(), value.as_str().unwrap().to_string()))
        .collect();
    expected.sort();
    assert_eq!(fields, expected, "{name}");
}

fn status(answer: &Value) -> u16 {
    u16::try_from(answer["status"].as_u64().unwrap()).unwrap()
}

fn text(answer: &Value, field: &str) -> String {
    answer[field].as_str().unwrap().to_string()
}
