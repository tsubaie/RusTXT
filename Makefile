# Development gates. Run `make setup` once per clone to install the pre-push
# hook; from then on every push runs `make check`, the same steps CI runs.
.PHONY: check fmt-check fmt clippy test e2e build run setup

check: fmt-check clippy test

fmt-check:
	cargo fmt --all -- --check

fmt:
	cargo fmt --all

clippy:
	cargo clippy -p rustxt-core -p rustxt-iced --all-targets --locked -- -D warnings

test:
	cargo test -p rustxt-core -p rustxt-iced --locked

# The end-to-end tests alone. They open real windows, so they need a display.
e2e: build
	xvfb-run -a -s '-screen 0 1280x1024x24' python3 tools/iced-e2e.py target/release/rustxt-iced target/iced-e2e

legacy-e2e:
	cargo test -p rustxt --locked --test e2e

build:
	cargo build --release --locked -p rustxt-iced

run:
	cargo run -p rustxt-iced

setup:
	git config core.hooksPath .githooks
	@echo "Pre-push hook installed: every push now runs 'make check'."
