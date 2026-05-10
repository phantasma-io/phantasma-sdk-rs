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

package-dirty:
    cargo package --allow-dirty

package:
    cargo package

verify: fmt-check check test clippy doc

release-check:
    test -z "$(git status --porcelain)" || (git status --short && false)
    just verify
    cargo package
    cargo publish --dry-run

publish-dry-run:
    just release-check

publish:
    just release-check
    cargo publish
