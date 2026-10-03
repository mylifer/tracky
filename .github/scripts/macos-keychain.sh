#!/usr/bin/env bash
# Kum'un kendinden imzalı sertifikasını (MACOS_CERTIFICATE secret'ı) geçici bir
# anahtar zincirine aktarır ve tauri'nin imzalaması için APPLE_SIGNING_IDENTITY
# ayarlar. Secret yoksa hiçbir şey yapmaz; paket ad-hoc imzalanır.
set -euo pipefail

if [ -z "${MACOS_CERTIFICATE:-}" ]; then
  echo "MACOS_CERTIFICATE yok: paket ad-hoc imzalanacak (izinler her sürümde yeniden istenir)."
  exit 0
fi

keychain="$RUNNER_TEMP/kum-signing.keychain-db"
password="$(openssl rand -hex 16)"
p12="$RUNNER_TEMP/kum-codesign.p12"
echo "$MACOS_CERTIFICATE" | base64 --decode > "$p12"

security create-keychain -p "$password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$password" "$keychain"
security import "$p12" -k "$keychain" -P "$MACOS_CERTIFICATE_PASSWORD" -T /usr/bin/codesign
rm -f "$p12"
# codesign anahtarı parola penceresi açmadan kullanabilsin.
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$password" "$keychain" > /dev/null
# Mevcut zincirlerin önüne ekle; codesign kimliği arama listesinde bulur.
existing=$(security list-keychains -d user | sed -e 's/^[[:space:]]*"//' -e 's/"$//')
# shellcheck disable=SC2086
security list-keychains -d user -s "$keychain" $existing

# Kendinden imzalı sertifika "güvenilmez" görünür (-v ile listelenmez) ama
# codesign onunla imzalar; kimlik SHA-1 özetiyle seçilir.
security find-identity -p codesigning "$keychain"
identity=$(security find-identity -p codesigning "$keychain" | awk '/[0-9]\)/ { print $2; exit }')
if [ -z "$identity" ]; then
  echo "Sertifikada kod imzalama kimliği bulunamadı." >&2
  exit 1
fi
echo "APPLE_SIGNING_IDENTITY=$identity" >> "$GITHUB_ENV"
echo "İmza kimliği: $identity"
