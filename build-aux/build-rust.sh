#!/bin/sh
# Meson only installs the Cargo-built executable and desktop metadata.
set -eu
source_dir=$1
output=$2
build_dir=$3
target_dir=${CARGO_TARGET_DIR:-$build_dir/cargo-target}
cargo build --manifest-path "$source_dir/Cargo.toml" --release --locked --bin paper-trail --target-dir "$target_dir"
cp "$target_dir/release/paper-trail" "$output"
