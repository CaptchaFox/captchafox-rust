## 1.0.0

- Verify CaptchaFox tokens with siteverify: `Client::verify` returns `Verification::Passed` or `Verification::Rejected`,
  or `VerificationUnavailable` when CaptchaFox cannot be reached in time or answers unexpectedly.
- Rust 1.85 and later, on Tokio 1.
