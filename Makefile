BINSTALL := $(shell command -v cargo-binstall 2> /dev/null)

SUPPORTED_ARGS := check test
FIRST_WORD := $(firstword $(MAKECMDGOALS))
ifneq ($(filter $(FIRST_WORD),$(SUPPORTED_ARGS)),)
  CMD_ARGS := $(filter-out $(FIRST_WORD),$(MAKECMDGOALS))
  $(eval $(CMD_ARGS):;@:)
endif

.PHONY: setup-binstall
setup-binstall:
ifndef BINSTALL
	@echo "Installing cargo-binstall..."
ifeq ($(OS),Windows_NT)
	@powershell -c "iex (irm https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.ps1)"
else
	@curl --proto '=https' --tlsv1.2 -LsSf https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash
endif
endif

.PHONY: setup
setup: setup-binstall
	@cargo binstall -y cargo-nextest cargo-edit cargo-sort cargo-machete cargo-tarpaulin

.PHONY: format
format:
	cargo +nightly fmt --all

.PHONY: check
check: format
	cargo +nightly clippy $(filter-out --,$(CMD_ARGS)) --all-features --all-targets --fix --allow-dirty -- -D warnings

.PHONY: test
test: check
	cargo nextest run --config-file nextest.toml $(CMD_ARGS)

.PHONY: bench
bench: check
	cargo bench

.PHONY: upgrade
upgrade:
	cargo upgrade --incompatible && cargo sort -w

.PHONY: machete
machete:
	cargo machete --with-metadata

.PHONY: tarpaulin
tarpaulin:
	cargo tarpaulin --all-targets -- --test-threads 1 --no-capture
