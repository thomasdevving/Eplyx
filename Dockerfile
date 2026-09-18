# syntax=docker/dockerfile:1
#
# Hosted Eplyx CI API.
#
# The serving path is offline: no RPC endpoint, no archive credentials, and no
# outbound request of any kind. The runtime image therefore carries the binary
# and nothing else — not even a CA bundle, which would imply a network client
# that does not exist.
#
# Candidate code is never built here. This image compiles the server; the `.so`
# under test arrives as bytes over HTTP and executes only inside the replay VM
# the engine already sandboxes.

FROM rust:1-bookworm AS builder
WORKDIR /build

# The toolchain file pins the channel. Resolving it in its own layer keeps a
# source change from re-downloading the compiler.
COPY rust-toolchain.toml ./
RUN rustup show

# Dependencies first. The litesvm and solana-* tree dominates this build and
# moves only when the lockfile does, so it is compiled against stub sources and
# cached independently of the engine's own code.
#
# Cargo validates every *explicitly declared* target before it compiles
# anything, so each one needs a stub file even when this build would never
# compile it: `engine`'s `[[bin]] eplyx` and `server`'s `[lib] eplyx_server`.
# A missing one fails the layer with "can't find lib", not with a type error.
COPY Cargo.toml Cargo.lock ./
COPY interface/Cargo.toml interface/
COPY engine/Cargo.toml engine/
COPY server/Cargo.toml server/
RUN mkdir -p interface/src engine/src server/src \
 && : > interface/src/lib.rs \
 && : > engine/src/lib.rs \
 && echo 'fn main() {}' > engine/src/main.rs \
 && : > server/src/lib.rs \
 && echo 'fn main() {}' > server/src/main.rs \
 && cargo build --release -p eplyx-server \
 && rm -rf interface/src engine/src server/src

COPY . .
# Cargo decides what to rebuild from mtimes, and the real sources arrive with
# the build context's timestamps, which can predate the stub build above. Every
# stubbed file is listed so the set stays obvious next to the one above.
RUN touch interface/src/lib.rs engine/src/lib.rs engine/src/main.rs \
      server/src/lib.rs server/src/main.rs \
 && cargo build --release -p eplyx-server

FROM debian:bookworm-slim AS runtime

# A bundle baked in from deploy/bundle, if one was placed there. It is present,
# not installed and not active: `admin install-bundle` and `admin
# activate-bundle` stay deliberate operator steps, because activating a bundle
# moves what every pull request is measured against.
COPY deploy/bundle /opt/eplyx/bundle
COPY --from=builder /build/target/release/eplyx-server /usr/local/bin/eplyx-server

# Runs as root so the mounted volume is writable. A managed host attaches its
# volume owned by root, and a dropped privilege would leave the server unable to
# store a project, a bundle or a run.
ENV EPLYX_DATA_DIR=/data
EXPOSE 8080
CMD ["eplyx-server", "serve"]
