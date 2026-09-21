# U7 raw Yellowstone capture

`stream.pbseq` is a sequence of little-endian `uint32` byte lengths followed by serialized `SubscribeUpdate` protobuf messages. `stream-index.json` gives event kind, slot, byte length, and SHA-256 for every frame. `stream-receipt.json` gives the credential-free request contract and transport result.

The pinned source schema is [`../phase-u6-freeze/geyser.proto`](../phase-u6-freeze/geyser.proto), commit `7139edd23c44470b4d260fabd5c270907014c270`. Generate Python `geyser_pb2` and `geyser_pb2_grpc` modules from this proto and [its matching `solana-storage.proto`](https://github.com/rpcpool/yellowstone-grpc/blob/7139edd23c44470b4d260fabd5c270907014c270/yellowstone-grpc-proto/proto/solana-storage.proto), then run `scripts/analyze-u7-yellowstone.py` with `--stream`, `--block docs/examples/phase-u5-sample/blocks/448760958.body`, `--inventory docs/examples/phase-u6-freeze/account-boundary-status.json`, `--slot 448760958`, and `--out` in a temporary directory. The retained `analysis.json` is the result of that offline analysis.

The historical account feed is proven coalesced for at least one address in this slot. These frames must not be interpreted as a complete sequence of account writes.
