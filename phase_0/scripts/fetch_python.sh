#!/usr/bin/env bash
# Download and unpack the official python.org Android releases (PEP 738).
#
# These are *official binary releases* from python.org — the thing plan1.md §1 says
# removes the need to maintain a patched CPython fork. Nothing is compiled here.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VENDOR="$(dirname "$HERE")/vendor"
: "${PY_FULL:=3.14.6}"
BASE="https://www.python.org/ftp/python/$PY_FULL"

mkdir -p "$VENDOR"
cd "$VENDOR"

for ARCH in aarch64 x86_64; do
  TARBALL="python-$PY_FULL-$ARCH-linux-android.tar.gz"
  DEST="python-$ARCH"
  if [ -f "$DEST/prefix/lib/libpython3.14.so" ]; then
    echo "$DEST already unpacked"
    continue
  fi
  if [ ! -f "$TARBALL" ]; then
    echo "==> downloading $TARBALL"
    curl -fSL --progress-bar -o "$TARBALL" "$BASE/$TARBALL"
  fi
  echo "==> unpacking into $VENDOR/$DEST"
  mkdir -p "$DEST"
  tar xzf "$TARBALL" -C "$DEST"
done

python3 "$HERE/../../scripts/fetch_pack.py"

echo
echo "vendor/ ready:"
du -sh "$VENDOR"/python-* "$VENDOR/pack" 2>/dev/null | sed 's/^/  /'
