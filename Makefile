# Upgrade Impact CI - developer entry points.
#
# `make` builds both program versions and runs the differential comparison.

SHELL := /bin/bash
CARGO := cargo

.PHONY: all build programs compare report fixtures test test-engine test-programs fmt fmt-check lint clean demo-replay demo-discovery demo-mainnet-replay demo-token2022-upgrade demo-cpi-mainnet-replay memo-candidate token2022-candidate stake-pool-candidate

all: compare

## Compile V1 and V2 to SBF bytecode.
programs build:
	./scripts/build-programs.sh

## Build both versions and report what changed. The headline command.
compare: build
	$(CARGO) run -q -p eplyx-engine -- compare

## Same, as machine-readable JSON for a CI gate.
report: build
	$(CARGO) run -q -p eplyx-engine -- compare --format json --out report.json

## Regenerate the checked-in fixture corpus.
fixtures:
	$(CARGO) run -q -p eplyx-engine -- generate

## Everything: program unit tests plus the differential suite.
#
# The stake-pool candidate is built here because the Phase 8 CPI execution tests
# load both of its flavours. Everything that executes needs its artefact present;
# the tests fail with an explanatory message rather than skipping.
test: build stake-pool-candidate test-programs test-engine

test-engine:
	$(CARGO) test

test-programs:
	./scripts/test-programs.sh

fmt:
	$(CARGO) fmt --all
	$(CARGO) fmt --all --manifest-path programs/fixture-lending/Cargo.toml
	$(CARGO) fmt --all --manifest-path programs/fixture-memo-candidate/Cargo.toml
	$(CARGO) fmt --all --manifest-path programs/fixture-token2022-candidate/Cargo.toml
	$(CARGO) fmt --all --manifest-path programs/fixture-stake-pool-candidate/Cargo.toml

fmt-check:
	$(CARGO) fmt --all -- --check
	$(CARGO) fmt --all --manifest-path programs/fixture-lending/Cargo.toml -- --check
	$(CARGO) fmt --all --manifest-path programs/fixture-memo-candidate/Cargo.toml -- --check
	$(CARGO) fmt --all --manifest-path programs/fixture-token2022-candidate/Cargo.toml -- --check
	$(CARGO) fmt --all --manifest-path programs/fixture-stake-pool-candidate/Cargo.toml -- --check

lint:
	$(CARGO) clippy --all-targets -- -D warnings
	$(CARGO) clippy --manifest-path programs/fixture-lending/Cargo.toml \
		--no-default-features --features v1 --all-targets -- -D warnings
	$(CARGO) clippy --manifest-path programs/fixture-lending/Cargo.toml \
		--no-default-features --features v2 --all-targets -- -D warnings
	$(CARGO) clippy --manifest-path programs/fixture-memo-candidate/Cargo.toml \
		--all-targets -- -D warnings
	$(CARGO) clippy --manifest-path programs/fixture-token2022-candidate/Cargo.toml \
		--all-targets -- -D warnings
	$(CARGO) clippy --manifest-path programs/fixture-stake-pool-candidate/Cargo.toml \
		--all-targets -- -D warnings
	$(CARGO) clippy --manifest-path programs/fixture-stake-pool-candidate/Cargo.toml \
		--features reference --all-targets -- -D warnings

clean:
	$(CARGO) clean
	$(CARGO) clean --manifest-path programs/fixture-lending/Cargo.toml
	$(CARGO) clean --manifest-path programs/fixture-memo-candidate/Cargo.toml
	$(CARGO) clean --manifest-path programs/fixture-token2022-candidate/Cargo.toml
	$(CARGO) clean --manifest-path programs/fixture-stake-pool-candidate/Cargo.toml
	rm -rf artifacts report.json

## Real local-validator capture, RPC ingestion and offline replay demonstration.
demo-replay:
	./scripts/demo-real-replay.sh

## Bounded real-mainnet discovery, selection, and deterministic cache replay.
demo-discovery:
	./scripts/demo-mainnet-discovery.sh

## Exact archived mainnet replay and deliberately regressed Memo candidate.
demo-mainnet-replay:
	./scripts/demo-mainnet-replay.sh

## Real Token-2022 upgrade replayed over a real historical PYUSD transfer.
demo-token2022-upgrade:
	./scripts/demo-token2022-upgrade.sh

## Real stake-pool upgrade replayed over a real historical CPI deposit.
demo-cpi-mainnet-replay:
	./scripts/demo-stake-pool-upgrade.sh

memo-candidate:
	./scripts/build-memo-candidate.sh

token2022-candidate:
	./scripts/build-token2022-candidate.sh

stake-pool-candidate:
	./scripts/build-stake-pool-candidate.sh
