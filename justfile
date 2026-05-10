default: verify

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

check:
    cargo check --all-targets

test:
    cargo test --all-targets --no-fail-fast

clippy:
    cargo clippy --all-targets -- -D warnings

doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps

package:
    cargo package --allow-dirty

verify: fmt-check check test clippy doc
