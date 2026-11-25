#!/usr/bin/env bash
# Builds komga-rust for one target and packages it as dist/komga-rust-<version>-<target>.tar.gz
# (.zip for Windows, via 7z).
# Cross linkers and C compilers come from the caller's environment (CC_<target>,
# CARGO_TARGET_<TARGET>_LINKER, ...). cargo-about must be on PATH.
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <rust-target> <version>" >&2
  exit 2
fi
target=$1
version=$2

# cargo resolves .cargo/config.toml (tokio_unstable) from the cwd, not --manifest-path
cd "$(dirname "$0")/../.."
here=tools/release
name="komga-rust-$version-$target"
stage="dist/$name"
exe=komga-rust
archive="$stage.tar.gz"
if [[ $target == *windows* ]]; then
  exe=komga-rust.exe
  archive="$stage.zip"
fi

VERSION_NEXT=$version cargo build --locked --release --target "$target" -p komga-rust

rm -rf "$stage" "$archive"
mkdir -p "$stage"
cp "${CARGO_TARGET_DIR:-target}/$target/release/$exe" "$stage/"
cp ../LICENSE "$stage/LICENSE"
cp "$here/OpenDyslexic-OFL.txt" "$stage/"
cargo about generate --locked --fail --target "$target" \
  -c "$here/about.toml" -m crates/komga-rust/Cargo.toml \
  -o "$stage/THIRD-PARTY-LICENSES.html" "$here/about.hbs"

if [[ $archive == *.zip ]]; then
  (cd dist && 7z a -tzip -bso0 -bsp0 "$name.zip" "$name")
else
  tar -C dist -czf "$archive" "$name"
fi
echo "$PWD/$archive"
