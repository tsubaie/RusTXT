# Development gates. Run `make setup` once per clone to install the pre-push
# hook; from then on every push runs `make check`, the same steps CI runs.
.PHONY: check fmt-check fmt clippy test e2e build run setup

check: fmt-check clippy test

fmt-check:
	cargo fmt --all -- --check

fmt:
	cargo fmt --all

clippy:
	cargo clippy --workspace --all-targets --locked -- -D warnings

test:
	cargo test --workspace --locked

# The end-to-end tests alone. They open real windows, so they need a display.
e2e:
	cargo test -p rustxt --locked --test e2e

build:
	cargo build --release --locked -p rustxt

run:
	cargo run -p rustxt

setup:
	git config core.hooksPath .githooks
	@echo "Pre-push hook installed: every push now runs 'make check'."
