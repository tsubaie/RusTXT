# Development gates. Run `make setup` once per clone to install the pre-push
# hook; from then on every push runs `make check`, the same steps CI runs.
.PHONY: check fmt-check fmt clippy test e2e goldens goldens-update build run setup

check: fmt-check clippy test

fmt-check:
	cargo fmt --all -- --check

fmt:
	cargo fmt --all

clippy:
	cargo clippy -p rustxt-core -p rustxt-iced --all-targets --locked -- -D warnings

test:
	cargo test -p rustxt-core -p rustxt-iced --locked
	cargo test -p iced_tiny_skia --lib --locked

# The end-to-end tests alone. They open real windows, so they need a display.
e2e: build
	xvfb-run -a -s '-screen 0 1280x1024x24' python3 tools/iced-e2e.py target/release/rustxt-iced target/iced-e2e
	xvfb-run -a -s '-screen 0 1600x1200x24' python3 tools/iced-rendering-e2e.py target/release/rustxt-iced target/iced-rendering-e2e
	xvfb-run -a -s '-screen 0 1280x1024x24' python3 tools/iced-regressions-e2e.py target/release/rustxt-iced target/iced-regressions

# Golden screenshots, compared against tools/goldens (made on the CI image, so
# not part of e2e; see tools/goldens/README.md). goldens-update rewrites them.
goldens: build
	xvfb-run -a -s '-screen 0 1600x1200x24' python3 tools/iced-golden-e2e.py target/release/rustxt-iced target/iced-golden

goldens-update: build
	xvfb-run -a -s '-screen 0 1600x1200x24' python3 tools/iced-golden-e2e.py target/release/rustxt-iced target/iced-golden --update

build:
	cargo build --release --locked -p rustxt-iced

run:
	cargo run -p rustxt-iced

setup:
	git config core.hooksPath .githooks
	@echo "Pre-push hook installed: every push now runs 'make check'."
