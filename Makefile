# Run inside the nix dev shell: `nix develop --command make check`.

.PHONY: check fmt fmt-check lint test e2e build run clean

# Verifies the entire project. Every merge must pass this.
check: fmt-check lint test

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all --check

lint:
	cargo clippy --workspace --all-targets -- -D warnings

# Runs every test in the workspace, including the Cucumber e2e suite.
test:
	cargo test --workspace

# Runs only the Cucumber e2e suite.
e2e:
	cargo test --package cutaway-e2e --test cucumber

build:
	cargo build --workspace

run:
	cargo run --package cutaway

clean:
	cargo clean
