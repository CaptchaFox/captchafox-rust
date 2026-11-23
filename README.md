# CaptchaFox for Rust

Official [CaptchaFox](https://captchafox.com) integration for Rust. It verifies the answers of the widget with
CaptchaFox.

## Compatibility

| Rust | Library | Runtime |
|---|---|---|
| 1.85 and later | 1.x | Tokio 1 |

Rust 1.84 and older are not supported: Cargo refuses to build the library there.

## Installation

```sh
cargo add captchafox
```

or in `Cargo.toml`:

```toml
[dependencies]
captchafox = "1"
```

The latest version is on [crates.io](https://crates.io/crates/captchafox).

## Configuration

### Get your keys

In the [CaptchaFox portal](https://portal.captchafox.com), open your website under **Sites > Site Details** and copy
the **site key** and the **secret key**.

### Set up the library

Create a `Client` once with the secret key of your site and reuse it; it can be shared between tasks, and its clones
share its configuration. Keep the secret key in your configuration or environment, not in source code:

```rust
let client = captchafox::Client::new(std::env::var("CAPTCHAFOX_SECRET")?);
```

Verifications run on Tokio: call them within a Tokio runtime with I/O and time enabled, as `#[tokio::main]` sets up.

### Options

| Option | Default | Meaning |
|---|---|---|
| `timeout` | 5 seconds | How long a verification may take, from connecting to CaptchaFox to the end of its answer: `Client::with_timeout(secret, Duration::from_secs(3))` |

## Usage

The widget adds the token to the form as the POST parameter `cf-captcha-response`. Verify it on the server before
the form is processed:

```rust
use captchafox::Verification;

match client.verify_with(&token, SITE_KEY, &remote_ip).await {
    Ok(Verification::Passed { .. }) => { /* Process the form. */ }
    Ok(Verification::Rejected { .. }) => { /* Reject the form. */ }
    Err(unavailable) => { /* CaptchaFox gave no usable answer: decide whether the form passes. */ }
}
```

`verify(token)` sends the token alone; `verify_with(token, sitekey, remote_ip)` also sends the site key the token was
issued for and the visitor's IP address, each only when it is not empty.

A verification has one of three outcomes:

- `Verification::Passed`: CaptchaFox accepted the token. `challenge_ts` and `hostname` hold the time of the challenge
  and the hostname of the site, where CaptchaFox sent them.
- `Verification::Rejected`: CaptchaFox rejected the token; `error_codes` holds its error codes, such as
  `timeout-or-duplicate`, possibly none. An empty token is rejected with `missing-input-response` without asking
  CaptchaFox.
- `Err(VerificationUnavailable)`: CaptchaFox gave no usable answer; see
  [If CaptchaFox is unreachable](#if-captchafox-is-unreachable).

## Operation

### If CaptchaFox is unreachable

When CaptchaFox cannot be reached, does not complete its answer within the timeout, answers with an HTTP status other
than 200, or with a body that is no valid answer, `verify` returns `Err(VerificationUnavailable)`. Your code decides
whether the form passes then: letting it through keeps your forms available during an outage, rejecting it keeps them
protected. Tokens that CaptchaFox rejects are always `Verification::Rejected`.

### Log

The library logs nothing. The messages of its errors never contain the secret key, and neither does the debug form of
`Client`.

### Visitor IP

The visitor's IP address is sent only when you pass it as `remote_ip`. Behind a reverse proxy or load balancer, pass
the visitor's address that the proxy forwards, not the address of the proxy.

### Privacy

CaptchaFox is operated by Scoria Labs GmbH, Germany. How CaptchaFox processes data and how to get a data processing
agreement is described in the [CaptchaFox Privacy Center](https://captchafox.com/privacy). A suggested paragraph for
your privacy policy is on the [GDPR page](https://captchafox.com/privacy/gdpr).

## License

Apache-2.0. See [LICENSE](LICENSE).
