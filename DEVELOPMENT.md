# Development

## Requirements

Rust 1.85 or later with Cargo, and a C compiler, with which the TLS dependency `ring` builds its cryptography. On
Rust 1.85, set `CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback`, so that Cargo chooses the versions of the
dependencies that still support it, as CI does.

## Test

| Command | Runs |
|---|---|
| `cargo test` | The tests against a local siteverify server, including every case of `src/tests/__fixtures__/siteverify-cases.json` and the reading of malformed HTTP answers, and the test of the public interface in `tests/public_interface.rs`. Needs no network once Cargo has downloaded the dependencies. CI runs it on Rust 1.85 and the latest stable for every push to `main` and every pull request. |
| `cargo test --test live -- --ignored` | The test against `https://api.captchafox.com/siteverify` with the public test keys. Needs the network. |
