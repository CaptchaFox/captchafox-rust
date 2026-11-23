//! How the answer of siteverify is delimited: an answer whose framing is broken or ambiguous is never evaluated.

use std::time::Duration;

use super::local_siteverify::{LocalSiteverify, Mode};
use crate::{Client, Verification, VerificationUnavailable};

const PASS: &str = r#"{"success":true}"#;

async fn answer(raw: &[u8]) -> Result<Verification, VerificationUnavailable> {
    let server = LocalSiteverify::start(Mode::Raw(raw.to_vec())).await;
    let client = Client::local(
        "ok_11111111000000001111111100000000",
        Duration::from_secs(2),
        server.address(),
    );
    client.verify("token").await
}

async fn passes(raw: &[u8]) -> bool {
    matches!(answer(raw).await, Ok(Verification::Passed { .. }))
}

async fn unavailable(raw: &[u8]) -> bool {
    answer(raw).await.is_err()
}

#[tokio::test]
async fn a_body_by_content_length_is_read() {
    assert!(
        passes(format!("HTTP/1.1 200 OK\r\nContent-Length: 16\r\n\r\n{PASS}").as_bytes()).await
    );
    assert!(
        passes(format!("HTTP/1.1 200 OK\r\ncontent-length:  16 \r\n\r\n{PASS}").as_bytes()).await
    );
}

#[tokio::test]
async fn only_the_bytes_of_content_length_are_the_body() {
    assert!(
        passes(format!("HTTP/1.1 200 OK\r\nContent-Length: 16\r\n\r\n{PASS}trailing").as_bytes())
            .await
    );
    assert!(
        unavailable(format!("HTTP/1.1 200 OK\r\nContent-Length: 15\r\n\r\n{PASS}").as_bytes())
            .await
    );
}

#[tokio::test]
async fn a_body_shorter_than_content_length_cannot_be_evaluated() {
    assert!(
        unavailable(format!("HTTP/1.1 200 OK\r\nContent-Length: 17\r\n\r\n{PASS}").as_bytes())
            .await
    );
}

#[tokio::test]
async fn a_content_length_that_is_no_single_number_cannot_be_evaluated() {
    for length in [
        "+16",
        "16, 16",
        "0x10",
        "",
        "16 16",
        "99999999999999999999999",
    ] {
        let raw = format!("HTTP/1.1 200 OK\r\nContent-Length: {length}\r\n\r\n{PASS}");
        assert!(
            unavailable(raw.as_bytes()).await,
            "Content-Length: {length}"
        );
    }
    let repeated =
        format!("HTTP/1.1 200 OK\r\nContent-Length: 16\r\nContent-Length: 16\r\n\r\n{PASS}");
    assert!(unavailable(repeated.as_bytes()).await);
}

#[tokio::test]
async fn a_chunked_body_is_read() {
    let raw = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
               b;name=value\r\n{\"success\":\r\n5\r\ntrue}\r\n0\r\nTrailer: x\r\n\r\n";
    assert!(passes(raw.as_bytes()).await);

    let upper =
        format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: Chunked\r\n\r\n10 \r\n{PASS}\r\n0\r\n\r\n");
    assert!(passes(upper.as_bytes()).await);
}

#[tokio::test]
async fn a_chunked_body_without_its_last_chunk_cannot_be_evaluated() {
    let base = format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n10\r\n{PASS}\r\n");
    assert!(unavailable(base.as_bytes()).await);
    assert!(unavailable(format!("{base}0\r\n").as_bytes()).await);
    assert!(
        unavailable(
            format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n11\r\n{PASS}").as_bytes()
        )
        .await
    );
}

#[tokio::test]
async fn a_chunk_that_breaks_its_framing_cannot_be_evaluated() {
    let head = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n";
    for chunks in [
        format!("zz\r\n{PASS}\r\n0\r\n\r\n"),
        format!("-10\r\n{PASS}\r\n0\r\n\r\n"),
        format!("10x\r\n{PASS}\r\n0\r\n\r\n"),
        format!("ffffffffffffffffffff\r\n{PASS}\r\n0\r\n\r\n"),
        // Valid chunks, if what follows a chunk were not checked to be CRLF.
        format!("10\r\n{PASS}XX0\r\n\r\n"),
        format!("10\n{PASS}\n0\n\n"),
    ] {
        assert!(
            unavailable(format!("{head}{chunks}").as_bytes()).await,
            "{chunks:?}"
        );
    }
}

#[tokio::test]
async fn an_ambiguous_framing_cannot_be_evaluated() {
    // A valid chunked body, so that only the refusal of the two framings together makes it unavailable.
    let both = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: 16\r\nTransfer-Encoding: chunked\r\n\r\n10\r\n{PASS}\r\n0\r\n\r\n"
    );
    assert!(unavailable(both.as_bytes()).await);

    let gzip = format!(
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip, chunked\r\n\r\n10\r\n{PASS}\r\n0\r\n\r\n"
    );
    assert!(unavailable(gzip.as_bytes()).await);

    let repeated = format!(
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nTransfer-Encoding: chunked\r\n\r\n10\r\n{PASS}\r\n0\r\n\r\n"
    );
    assert!(unavailable(repeated.as_bytes()).await);
}

#[tokio::test]
async fn a_body_without_length_ends_with_the_connection() {
    assert!(
        passes(
            format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{PASS}").as_bytes()
        )
        .await
    );
    assert!(passes(format!("HTTP/1.0 200 OK\r\n\r\n{PASS}").as_bytes()).await);
}

#[tokio::test]
async fn an_answer_larger_than_its_bound_is_not_cut_off_but_cannot_be_evaluated() {
    // Valid JSON up to any cut, so that a reader that stops at its bound would pass it.
    let raw = format!("HTTP/1.1 200 OK\r\n\r\n{PASS}{}", " ".repeat(1024 * 1024));
    assert!(unavailable(raw.as_bytes()).await);

    let declared = format!("HTTP/1.1 200 OK\r\nContent-Length: 1048577\r\n\r\n{PASS}");
    assert!(unavailable(declared.as_bytes()).await);
}

#[tokio::test]
async fn a_head_that_breaks_http_cannot_be_evaluated() {
    for head in [
        "HTTP/1.1 2000 OK\r\n",
        "HTTP/1.1 20 OK\r\n",
        "HTTP/2 200 OK\r\n",
        "ICY 200 OK\r\n",
        "HTTP/1.1 200 OK\r\nX-Folded: a\r\n b\r\n",
        "HTTP/1.1 200 OK\r\nContent-Length : 16\r\n",
        "HTTP/1.1 200 OK\r\nNo colon\r\n",
        "HTTP/1.1 200 OK\n",
    ] {
        assert!(
            unavailable(format!("{head}\r\n{PASS}").as_bytes()).await,
            "{head:?}"
        );
    }
}

#[tokio::test]
async fn an_answer_that_ends_in_its_head_cannot_be_evaluated() {
    assert!(unavailable(b"").await);
    assert!(unavailable(b"HTTP/1.1 200 OK\r\nContent-Length: 16\r\n").await);
}

#[tokio::test]
async fn a_status_line_without_reason_is_read() {
    assert!(passes(format!("HTTP/1.1 200\r\n\r\n{PASS}").as_bytes()).await);
}

#[tokio::test]
async fn a_status_other_than_200_cannot_be_evaluated_without_reading_the_body() {
    assert!(
        unavailable(
            format!("HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\n\r\n{PASS}").as_bytes()
        )
        .await
    );
    assert!(unavailable(format!("HTTP/1.1 201 Created\r\n\r\n{PASS}").as_bytes()).await);
}
