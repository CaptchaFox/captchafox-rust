//! Verifies the public test keys against CaptchaFox's siteverify; needs the network. Run with
//! `cargo test --test live -- --ignored`.

use captchafox::{Client, Verification};

const PASSING_SECRET: &str = "ok_11111111000000001111111100000000";
const FAILING_SECRET: &str = "ok_FFFFFFFF00000000FFFFFFFF00000000";
const SITEKEY: &str = "sk_11111111000000001111111100000000";
const TOKEN: &str = "11111111000000001111111100000000";

#[tokio::test]
#[ignore = "needs the network"]
async fn the_test_key_passes() {
    let verification = Client::new(PASSING_SECRET).verify(TOKEN).await.unwrap();

    assert!(
        matches!(verification, Verification::Passed { .. }),
        "{verification:?}"
    );
}

#[tokio::test]
#[ignore = "needs the network"]
async fn the_test_key_passes_with_sitekey_and_remote_ip() {
    let verification = Client::new(PASSING_SECRET)
        .verify_with(TOKEN, SITEKEY, "203.0.113.7")
        .await
        .unwrap();

    assert!(
        matches!(verification, Verification::Passed { .. }),
        "{verification:?}"
    );
}

#[tokio::test]
#[ignore = "needs the network"]
async fn the_failing_test_key_is_rejected() {
    let verification = Client::new(FAILING_SECRET).verify(TOKEN).await.unwrap();

    match verification {
        Verification::Rejected { error_codes, .. } => {
            assert_eq!(error_codes, ["timeout-or-duplicate"])
        }
        other => panic!("expected rejected, got {other:?}"),
    }
}
