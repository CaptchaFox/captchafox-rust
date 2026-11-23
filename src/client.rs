use std::collections::HashSet;
use std::fmt;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use serde_core::de::{Deserialize, Deserializer, Error as _, IgnoredAny, MapAccess, Visitor};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio_rustls::rustls::crypto::ring;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

use crate::http;
use crate::{Verification, VerificationUnavailable};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) const HOST: &str = "api.captchafox.com";
pub(crate) const PATH: &str = "/siteverify";

/// Verifies tokens of the CaptchaFox widget with siteverify, with the secret key of a site. Create it once and reuse
/// it; a clone shares its TLS configuration.
///
/// Its futures need a Tokio runtime with I/O and time enabled, as `#[tokio::main]` sets up.
#[derive(Clone)]
pub struct Client {
    secret: String,
    timeout: Duration,
    endpoint: Endpoint,
}

#[derive(Clone)]
enum Endpoint {
    Siteverify(Arc<ClientConfig>),
    // Plain HTTP exists only in the tests, so that the secret key never travels unencrypted.
    #[cfg(test)]
    Local(std::net::SocketAddr),
}

impl Client {
    /// Gives a verification 5 seconds, from connecting to siteverify to the end of its answer.
    ///
    /// # Panics
    ///
    /// If the secret key is empty.
    pub fn new(secret: impl Into<String>) -> Client {
        Client::with_timeout(secret, DEFAULT_TIMEOUT)
    }

    /// Gives a verification the timeout instead of 5 seconds, from connecting to siteverify to the end of its answer.
    ///
    /// # Panics
    ///
    /// If the secret key is empty or the timeout is zero.
    pub fn with_timeout(secret: impl Into<String>, timeout: Duration) -> Client {
        let secret = secret.into();
        assert!(!secret.is_empty(), "the secret key is empty");
        assert!(!timeout.is_zero(), "the timeout is zero");

        // The provider is chosen here, not taken from the process, so that the application may enable another one.
        let config = ClientConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("ring supports the default protocol versions")
            .with_root_certificates(RootCertStore::from_iter(
                webpki_roots::TLS_SERVER_ROOTS.iter().cloned(),
            ))
            .with_no_client_auth();

        Client {
            secret,
            timeout,
            endpoint: Endpoint::Siteverify(Arc::new(config)),
        }
    }

    #[cfg(test)]
    pub(crate) fn local(secret: &str, timeout: Duration, address: std::net::SocketAddr) -> Client {
        Client {
            endpoint: Endpoint::Local(address),
            ..Client::with_timeout(secret, timeout)
        }
    }

    #[cfg(test)]
    pub(crate) fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Verifies a token.
    ///
    /// An empty token is rejected with `missing-input-response` without a request. Returns
    /// [`VerificationUnavailable`] if siteverify cannot be evaluated.
    pub async fn verify(&self, token: &str) -> Result<Verification, VerificationUnavailable> {
        self.verify_with(token, "", "").await
    }

    /// Verifies a token that was issued for a site key to a visitor's IP address; each of the two is sent only when
    /// it is not empty.
    ///
    /// An empty token is rejected with `missing-input-response` without a request. Returns
    /// [`VerificationUnavailable`] if siteverify cannot be evaluated.
    pub async fn verify_with(
        &self,
        token: &str,
        sitekey: &str,
        remote_ip: &str,
    ) -> Result<Verification, VerificationUnavailable> {
        if token.is_empty() {
            return Ok(Verification::Rejected {
                error_codes: vec!["missing-input-response".to_string()],
            });
        }

        let mut form = String::new();
        append_field(&mut form, "secret", &self.secret);
        append_field(&mut form, "response", token);
        append_field(&mut form, "sitekey", sitekey);
        append_field(&mut form, "remoteIp", remote_ip);

        // One deadline over connect, TLS, request and the whole answer; dropping the exchange closes the connection.
        let body = match tokio::time::timeout(self.timeout, self.exchange(&form)).await {
            Ok(body) => body?,
            Err(_) => {
                return Err(VerificationUnavailable::new(format!(
                    "siteverify did not answer within {:?}",
                    self.timeout
                )));
            }
        };

        evaluate(&body)
    }

    async fn exchange(&self, form: &str) -> Result<Vec<u8>, VerificationUnavailable> {
        match &self.endpoint {
            Endpoint::Siteverify(config) => {
                let tcp = TcpStream::connect((HOST, 443)).await.map_err(not_reached)?;
                let mut tls = TlsConnector::from(Arc::clone(config))
                    .connect(ServerName::try_from(HOST).expect("HOST is a DNS name"), tcp)
                    .await
                    .map_err(not_reached)?;
                http::post_form(&mut tls, HOST, PATH, form).await
            }
            #[cfg(test)]
            Endpoint::Local(address) => {
                let mut tcp = TcpStream::connect(address).await.map_err(not_reached)?;
                http::post_form(&mut tcp, &address.to_string(), PATH, form).await
            }
        }
    }
}

// The secret key stays out of the debug form.
impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

fn not_reached(error: std::io::Error) -> VerificationUnavailable {
    VerificationUnavailable::caused_by("siteverify could not be reached", error)
}

/// `application/x-www-form-urlencoded` as browsers send it.
fn append_field(form: &mut String, name: &str, value: &str) {
    if value.is_empty() {
        return;
    }

    if !form.is_empty() {
        form.push('&');
    }
    form.push_str(name);
    form.push('=');
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                form.push(char::from(byte));
            }
            b' ' => form.push('+'),
            _ => {
                let _ = write!(form, "%{byte:02X}");
            }
        }
    }
}

pub(crate) fn evaluate(body: &[u8]) -> Result<Verification, VerificationUnavailable> {
    let Ok(text) = std::str::from_utf8(body) else {
        return Err(VerificationUnavailable::new(
            "siteverify answered with a body that is no UTF-8",
        ));
    };

    let answer: Answer = match serde_json::from_str(text) {
        Ok(answer) => answer,
        Err(error) if error.is_data() => {
            return Err(VerificationUnavailable::new(
                "siteverify answered with JSON that is no object or repeats a member",
            ));
        }
        // The message of serde_json is left out: it may quote the body.
        Err(_) => {
            return Err(VerificationUnavailable::new(
                "siteverify answered with a body that is no JSON",
            ));
        }
    };

    match answer.success {
        Some(Value::Bool(true)) => Ok(Verification::Passed {
            challenge_ts: string(answer.challenge_ts),
            hostname: string(answer.hostname),
        }),
        Some(Value::Bool(false)) => Ok(Verification::Rejected {
            error_codes: strings(answer.error_codes),
        }),
        _ => Err(VerificationUnavailable::new(
            "siteverify answered without a boolean success",
        )),
    }
}

/// The members of siteverify's answer that are needed.
struct Answer {
    success: Option<Value>,
    challenge_ts: Option<Value>,
    hostname: Option<Value>,
    error_codes: Option<Value>,
}

impl<'de> Deserialize<'de> for Answer {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Answer, D::Error> {
        deserializer.deserialize_map(AnswerVisitor)
    }
}

struct AnswerVisitor;

impl<'de> Visitor<'de> for AnswerVisitor {
    type Value = Answer;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut members: A) -> Result<Answer, A::Error> {
        let mut answer = Answer {
            success: None,
            challenge_ts: None,
            hostname: None,
            error_codes: None,
        };
        let mut names = HashSet::new();

        while let Some(name) = members.next_key::<String>()? {
            // A repeated name, such as "success" twice, has no defined meaning; serde_json would keep the last one.
            if !names.insert(name.clone()) {
                return Err(A::Error::custom("a member name is repeated"));
            }

            match name.as_str() {
                "success" => answer.success = Some(members.next_value()?),
                "challenge_ts" => answer.challenge_ts = Some(members.next_value()?),
                "hostname" => answer.hostname = Some(members.next_value()?),
                "error-codes" => answer.error_codes = Some(members.next_value()?),
                _ => {
                    members.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(answer)
    }
}

fn string(value: Option<Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) => Some(text),
        _ => None,
    }
}

// A rejection stays a rejection when its error codes are malformed; only the entries that are strings are kept.
fn strings(value: Option<Value>) -> Vec<String> {
    let Some(Value::Array(elements)) = value else {
        return Vec::new();
    };

    elements
        .into_iter()
        .filter_map(|element| match element {
            Value::String(text) => Some(text),
            _ => None,
        })
        .collect()
}
