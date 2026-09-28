#!/usr/bin/env python3
"""Check the canonical offline Stake Pool control and regression reports."""

import argparse
import hashlib
import json
from decimal import Decimal
from pathlib import Path

BUNDLE_SHA256 = "5e5b67ac13e4f6b8249348ad81db29885ee8ee897ba78f6de213b55793f4285f"
BASELINE_SHA256 = "ec2dfefaa70d560754a0000f39bd2cabc192b895d36205b3c428f601b6e1d7e1"
CANDIDATE_SHA256 = "3193eabd9fe2e479109ef3b2dd7301fffd06774325133ff8f88916ed482db099"
CONTROL_REPORT_SHA256 = "cb0e9d12290191ea7fd6a8c5ab02f226fe56ab35b20070a253f4e6e760014bef"
DEPOSIT = "spl-stake-pool/deposit_sol/economic/pool_tokens_received/decreased"
WITHDRAW = "spl-stake-pool/withdraw_sol/execution/transaction/now_reverts"
COVERAGE = {
    "spl-stake-pool/deposit_sol/economic/pool_tokens_received": 1,
    "spl-stake-pool/deposit_sol/execution/transaction": 1,
    "spl-stake-pool/withdraw_sol/economic/pool_tokens_burned": 9,
    "spl-stake-pool/withdraw_sol/economic/pool_tokens_debited": 9,
    "spl-stake-pool/withdraw_sol/economic/sol_received_by_user": 9,
    "spl-stake-pool/withdraw_sol/execution/transaction": 9,
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def read_report(path):
    try:
        data = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"cannot read valid JSON report {path}: {error}") from error
    require(isinstance(data, dict), f"{path}: expected a JSON object")
    require(data.get("schema_version") == 1, f"{path}: unexpected report schema")
    require(data.get("bundle", {}).get("sha256") == BUNDLE_SHA256, f"{path}: bundle identity changed")
    require(data["bundle"].get("record_count") == 10, f"{path}: expected ten records")
    coverage = {item["subject"]: item["observations"] for item in data.get("coverage", [])}
    require(coverage == COVERAGE, f"{path}: semantic coverage changed: {coverage}")
    require(data.get("unmatched") == [], f"{path}: unmatched declarations appeared")
    return data


def check_control(path):
    data = read_report(path)
    require(data.get("candidate", {}).get("sha256") == BASELINE_SHA256, "control candidate changed")
    require(data.get("findings") == [], "control has findings")
    summary = data.get("summary", {})
    require(summary.get("exit_code") == 0 and summary.get("passed") is True, "control did not pass")
    require(summary.get("unexpected") == 0 and data.get("failures") == [], "control has failures")
    actual_sha = hashlib.sha256(path.read_bytes()).hexdigest()
    require(actual_sha == CONTROL_REPORT_SHA256, f"control report bytes changed: {actual_sha}")
    print(f"control: 10 records, 6 covered subjects, 0 findings, SHA-256 {actual_sha}")


def check_regression(path):
    data = read_report(path)
    require(data.get("candidate", {}).get("sha256") == CANDIDATE_SHA256, "candidate identity changed")
    summary = data.get("summary", {})
    require(summary.get("exit_code") == 1 and summary.get("passed") is False, "regression did not fail as expected")
    reasons = {"undeclarable_change", "undeclared_change"}
    require(set(summary.get("failure_reasons", [])) == reasons, "regression failure reasons changed")
    require(set(data.get("failures", [])) == reasons, "regression failures changed")
    require(summary.get("unexpected") == 2, "expected exactly two unexpected findings")
    findings = data.get("findings", [])
    require(len(findings) == 2, f"expected exactly two finding categories, got {len(findings)}")
    by_id = {item["fingerprint"]: item for item in findings}
    require(set(by_id) == {DEPOSIT, WITHDRAW}, f"finding categories changed: {sorted(by_id)}")
    require(all(item.get("status") == "unexpected" for item in findings), "finding status changed")

    deposit = by_id[DEPOSIT]
    require(deposit.get("covered_observations") == 1, "DepositSol observation count changed")
    require(len(deposit.get("observations", [])) == 1, "DepositSol observation identity missing")
    values = deposit.get("values", [])
    require(len(values) == 1, "DepositSol economic values missing")
    before = Decimal(values[0]["baseline"]["quantity"])
    after = Decimal(values[0]["candidate"]["quantity"])
    require(after < before and values[0].get("relative_delta_bps") == -1, "DepositSol decrease changed")

    withdraw = by_id[WITHDRAW]
    observations = withdraw.get("observations", [])
    require(withdraw.get("covered_observations") == 9, "WithdrawSol count changed")
    require(len(observations) == len(set(observations)) == 9, "expected nine distinct WithdrawSol reverts")
    print("regression: 1 DepositSol decrease, 9 WithdrawSol new reverts, exactly 2 findings")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--control", required=True, type=Path)
    parser.add_argument("--regression", required=True, type=Path)
    parser.add_argument("--portable-control", required=True, type=Path)
    args = parser.parse_args()
    check_control(args.control)
    check_regression(args.regression)
    check_control(args.portable_control)
    require(
        args.portable_control.read_bytes() == args.control.read_bytes(),
        "portable control differs from in-checkout control",
    )
    print("portability: copied CLI produced byte-identical control report")


if __name__ == "__main__":
    try:
        main()
    except (KeyError, TypeError, ValueError, ArithmeticError) as error:
        raise SystemExit(f"stake-pool reproducibility: {error}") from error
