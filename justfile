[private]
just:
    just -l

[group('format')]
fmt:
    cargo fmt --all

[group('format')]
fmt-check:
    cargo fmt --all -- --check

[group('build')]
check:
    cargo check --all-targets

[group('test')]
test:
    cargo test --all-targets --no-fail-fast

[group('lint')]
clippy:
    cargo clippy --all-targets -- -D warnings

[group('doc')]
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps

[group('publish')]
package-dirty:
    cargo package --allow-dirty

[group('publish')]
package:
    cargo package

[group('test')]
verify: fmt-check check test clippy doc

[group('publish')]
release-check:
    test -z "$(git status --porcelain)" || (git status --short && false)
    just verify
    cargo package
    cargo publish --dry-run

[group('publish')]
publish-dry-run:
    just release-check

[group('publish')]
publish:
    just release-check
    cargo publish
