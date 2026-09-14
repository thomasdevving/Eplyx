# Upgrade Impact CI - developer entry points.
#
# `make` builds both program versions and runs the differential comparison.

SHELL := /bin/bash
CARGO := cargo

.PHONY: all build programs compare report fixtures test test-engine test-programs fmt fmt-check lint clean

all: compare

## Compile V1 and V2 to SBF bytecode.
programs build:
	./scripts/build-programs.sh

## Build both versions and report what changed. The headline command.
compare: build
	$(CARGO) run -q -p ripcord-engine -- compare

## Same, as machine-readable JSON for a CI gate.
report: build
	$(CARGO) run -q -p ripcord-engine -- compare --format json --out report.json

## Regenerate the checked-in fixture corpus.
fixtures:
	$(CARGO) run -q -p ripcord-engine -- generate

## Everything: program unit tests plus the differential suite.
test: build test-programs test-engine

test-engine:
	$(CARGO) test

test-programs:
	./scripts/test-programs.sh

fmt:
	$(CARGO) fmt --all
	$(CARGO) fmt --all --manifest-path programs/fixture-lending/Cargo.toml

fmt-check:
	$(CARGO) fmt --all -- --check
	$(CARGO) fmt --all --manifest-path programs/fixture-lending/Cargo.toml -- --check

lint:
	$(CARGO) clippy --all-targets -- -D warnings
	$(CARGO) clippy --manifest-path programs/fixture-lending/Cargo.toml \
		--no-default-features --features v1 --all-targets -- -D warnings
	$(CARGO) clippy --manifest-path programs/fixture-lending/Cargo.toml \
		--no-default-features --features v2 --all-targets -- -D warnings

clean:
	$(CARGO) clean
	$(CARGO) clean --manifest-path programs/fixture-lending/Cargo.toml
	rm -rf artifacts report.json
