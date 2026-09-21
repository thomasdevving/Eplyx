#!/usr/bin/env python3
"""Bounded, credential-free-on-disk capture of one historical Yellowstone slot.

The Alchemy key is read from a private file and used only as gRPC metadata. Raw
SubscribeUpdate protobuf messages are stored as little-endian length-prefixed
frames; the companion JSON index records their hashes and transport outcome.
Generated geyser_pb2 modules must be supplied on PYTHONPATH.
"""

import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import struct
import time

import grpc
import geyser_pb2
import geyser_pb2_grpc


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--key-file", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--slot", type=int, required=True)
    parser.add_argument("--accounts", type=Path, required=True)
    parser.add_argument("--all-accounts", action="store_true")
    parser.add_argument("--all-transactions", action="store_true")
    parser.add_argument("--include-full-block", action="store_true")
    parser.add_argument("--max-events", type=int, default=50000)
    parser.add_argument("--max-bytes", type=int, default=134217728)
    parser.add_argument("--timeout", type=int, default=120)
    args = parser.parse_args()

    key = args.key_file.read_text().strip()
    assert key and "\n" not in key
    addresses = [row["address"] for row in json.loads(args.accounts.read_text())["accounts"]]
    args.out.mkdir(parents=True, exist_ok=True)
    request = geyser_pb2.SubscribeRequest(
        from_slot=args.slot, commitment=geyser_pb2.FINALIZED
    )
    request.slots["finalized"].filter_by_commitment = True
    request.blocks_meta["all"].SetInParent()
    if args.all_transactions:
        request.transactions["all"].SetInParent()
    else:
        request.transactions["target_account"].account_include.append(
            "HxrqS96uHco2a8LxTvUwb9e2tS963Ec6nYkvZso9TDii"
        )
    if args.all_accounts:
        request.accounts["all"].SetInParent()
    else:
        request.accounts["target_keys"].account.extend(addresses)
    if args.include_full_block:
        request.blocks["all"].include_transactions = True
        request.blocks["all"].include_accounts = True
    receipt = {
        "endpoint_host": "solana-mainnet.streaming.alchemy.com",
        "requested_from_slot": args.slot,
        "commitment": "finalized",
        "account_filter_count": 0 if args.all_accounts else len(addresses),
        "transaction_filter": "all; vote/failed unset"
        if args.all_transactions
        else "mentions frozen Whirlpool; vote/failed unset",
        "full_block_requested": args.include_full_block,
        "request_sha256": hashlib.sha256(request.SerializeToString()).hexdigest(),
        "proto_commit": "7139edd23c44470b4d260fabd5c270907014c270",
        "max_events": args.max_events,
        "max_bytes": args.max_bytes,
        "timeout_seconds": args.timeout,
    }
    counts = Counter()
    slots = Counter()
    index = []
    total_bytes = 0
    finalized_target = False
    target_meta = False
    finalized_successor = False
    successor_meta = False
    channel = grpc.secure_channel(
        "solana-mainnet.streaming.alchemy.com:443", grpc.ssl_channel_credentials()
    )
    started = time.monotonic()
    stream = None
    try:
        stream = geyser_pb2_grpc.GeyserStub(channel).Subscribe(
            iter([request]), metadata=[("x-token", key)], timeout=args.timeout
        )
        with (args.out / "stream.pbseq").open("wb") as raw_out:
            for update in stream:
                raw = update.SerializeToString()
                kind = update.WhichOneof("update_oneof")
                event = getattr(update, kind)
                slot = getattr(event, "slot", None)
                raw_out.write(struct.pack("<I", len(raw)))
                raw_out.write(raw)
                total_bytes += len(raw) + 4
                counts[kind] += 1
                if slot is not None:
                    slots[str(slot)] += 1
                index.append(
                    {
                        "number": len(index),
                        "kind": kind,
                        "slot": slot,
                        "bytes": len(raw),
                        "sha256": hashlib.sha256(raw).hexdigest(),
                    }
                )
                if kind == "block_meta" and slot == args.slot:
                    target_meta = True
                if kind == "block_meta" and slot is not None and slot > args.slot:
                    successor_meta = True
                if kind == "slot" and event.status == geyser_pb2.SLOT_FINALIZED:
                    if slot == args.slot:
                        finalized_target = True
                    if slot is not None and slot > args.slot:
                        finalized_successor = True
                if (
                    target_meta
                    and finalized_target
                    and successor_meta
                    and finalized_successor
                ):
                    receipt["stop_reason"] = "target_and_successor_finalized_with_block_meta"
                    break
                if len(index) >= args.max_events or total_bytes >= args.max_bytes:
                    receipt["stop_reason"] = "capture_bound_reached"
                    break
            else:
                receipt["stop_reason"] = "server_stream_ended"
    except grpc.RpcError as error:
        receipt["stop_reason"] = "grpc_error"
        receipt["grpc_status"] = error.code().name
        receipt["grpc_details"] = (error.details() or "").replace(key, "[REDACTED]")
    finally:
        if stream is not None:
            stream.cancel()
        channel.close()
    receipt.update(
        {
            "elapsed_seconds": round(time.monotonic() - started, 3),
            "event_count": len(index),
            "frame_bytes": total_bytes,
            "frame_sha256": hashlib.sha256(
                (args.out / "stream.pbseq").read_bytes()
            ).hexdigest()
            if (args.out / "stream.pbseq").exists()
            else None,
            "counts_by_kind": dict(sorted(counts.items())),
            "counts_by_slot": dict(sorted(slots.items())),
            "target_block_meta_seen": target_meta,
            "target_finalized_seen": finalized_target,
            "successor_block_meta_seen": successor_meta,
            "successor_finalized_seen": finalized_successor,
        }
    )
    (args.out / "stream-index.json").write_text(
        json.dumps(index, indent=2, sort_keys=True) + "\n"
    )
    (args.out / "stream-receipt.json").write_text(
        json.dumps(receipt, indent=2, sort_keys=True) + "\n"
    )
    print(
        json.dumps(
            {k: receipt.get(k) for k in ("stop_reason", "grpc_status", "event_count", "frame_bytes", "counts_by_kind", "target_block_meta_seen", "target_finalized_seen")},
            sort_keys=True,
        ),
        flush=True,
    )


if __name__ == "__main__":
    main()
