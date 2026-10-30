#!/bin/sh
# Builds the static musl release binary with cargo-zigbuild and prints its size.
#
#   scripts/build-release.sh          production binary (no test hooks)
#   scripts/build-release.sh --bench  the same code with the `testhooks` feature, in its own
#                                     target dir, for the resource check (tests/loadcheck.rs)
#
# Needs: rustup with the x86_64-unknown-linux-musl target, zig, and
# `cargo install --locked cargo-zigbuild`. For an ARM mini PC set TARGET=aarch64-unknown-linux-musl.
set -eu

cd "$(dirname "$0")/.."

TARGET="${TARGET:-x86_64-unknown-linux-musl}"

case "${1:-}" in
	"")
		cargo zigbuild --release --target "$TARGET"
		BINARY="target/$TARGET/release/omron-mcp"
		;;
	--bench)
		cargo zigbuild --release --target "$TARGET" --features testhooks --target-dir target/bench
		BINARY="target/bench/$TARGET/release/omron-mcp"
		;;
	*)
		echo "usage: $0 [--bench]" >&2
		exit 2
		;;
esac

file "$BINARY"
ls -l "$BINARY"
echo "size: $(wc -c < "$BINARY") bytes"
