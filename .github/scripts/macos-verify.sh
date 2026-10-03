#!/usr/bin/env bash
# Paketlenen Kum.app'in imzasını gösterir. Sertifika verilmişse imza ad-hoc
# olmamalı: kalıcı izin için tasarım gereksinimi sertifikaya bağlı olmalı.
set -euo pipefail

app=$(find "${1:-target/release/bundle/macos}" -maxdepth 1 -name '*.app' | head -n 1)
if [ -z "$app" ]; then
  echo "Kum.app bulunamadı" >&2
  exit 1
fi
codesign --verify --deep --strict --verbose=2 "$app"
codesign -dv --verbose=2 "$app" 2>&1 | grep -E "^(Identifier|Authority|Signature|TeamIdentifier|Runtime)" || true
requirement=$(codesign -d -r- "$app" 2>&1 | grep "designated" || true)
echo "$requirement"
if [ -n "${MACOS_CERTIFICATE:-}" ] && echo "$requirement" | grep -q "cdhash"; then
  echo "Sertifika verildiği hâlde paket ad-hoc imzalanmış." >&2
  exit 1
fi
