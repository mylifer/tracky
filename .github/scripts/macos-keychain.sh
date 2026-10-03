#!/usr/bin/env bash
# Kum'un kendinden imzalı sertifikasını (MACOS_CERTIFICATE secret'ı) geçici bir
# anahtar zincirine aktarır ve tauri'nin imzalaması için APPLE_SIGNING_IDENTITY
# ayarlar. Secret yoksa hiçbir şey yapmaz; paket ad-hoc imzalanır.
#
# Hatalar GitHub uyarısı (annotation) olarak da yazılır: iş günlüğünü açmadan
# nedeni görülebilsin. Secret değerleri yazılmaz, yalnızca boyut ve özet.
set -euo pipefail

if [ -z "${MACOS_CERTIFICATE:-}" ]; then
  echo "MACOS_CERTIFICATE yok: paket ad-hoc imzalanacak (izinler her sürümde yeniden istenir)."
  exit 0
fi

fail() {
  # Çok satırlı ileti tek uyarıda kalsın.
  local msg="${1//'%'/'%25'}"
  msg="${msg//$'\r'/}"
  echo "::error title=macOS imza sertifikası::${msg//$'\n'/'%0A'}"
  exit 1
}

# Komutu çalıştırır; başarısızsa çıktısıyla birlikte uyarı yazar.
run() {
  local out
  if ! out=$("$@" 2>&1); then
    fail "$1 $2 başarısız:"$'\n'"$out"
  fi
  [ -n "$out" ] && echo "$out"
  return 0
}

keychain="$RUNNER_TEMP/kum-signing.keychain-db"
password="$(openssl rand -hex 16)"
p12="$RUNNER_TEMP/kum-codesign.p12"
# Yapıştırırken araya giren boşluk ve satır sonları değeri bozmasın.
cert_pw=$(printf '%s' "${MACOS_CERTIFICATE_PASSWORD:-}" | tr -d ' \t\r\n')
if ! printf '%s' "$MACOS_CERTIFICATE" | tr -d ' \t\r\n' | base64 --decode > "$p12" 2>/dev/null; then
  fail "MACOS_CERTIFICATE geçerli base64 değil (${#MACOS_CERTIFICATE} karakter). Değer eksik ya da bozuk kopyalanmış olabilir."
fi
echo "::notice title=macOS imza sertifikası::p12 $(wc -c < "$p12" | tr -d ' ') bayt, sha256 $(shasum -a 256 "$p12" | cut -c1-16); parola ${#cert_pw} karakter"
if ! openssl pkcs12 -in "$p12" -noout -passin "pass:$cert_pw" > /dev/null 2>&1 \
  && ! openssl pkcs12 -legacy -in "$p12" -noout -passin "pass:$cert_pw" > /dev/null 2>&1; then
  fail "p12 açılamadı: MACOS_CERTIFICATE_PASSWORD bu sertifikanın parolası değil ya da MACOS_CERTIFICATE bozuk."
fi

run security create-keychain -p "$password" "$keychain"
run security set-keychain-settings -lut 21600 "$keychain"
run security unlock-keychain -p "$password" "$keychain"
run security import "$p12" -k "$keychain" -P "$cert_pw" -T /usr/bin/codesign
rm -f "$p12"
# codesign anahtarı parola penceresi açmadan kullanabilsin.
run security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$password" "$keychain" > /dev/null
# Mevcut zincirlerin önüne ekle; codesign kimliği arama listesinde bulur.
existing=$(security list-keychains -d user | sed -e 's/^[[:space:]]*"//' -e 's/"$//')
# shellcheck disable=SC2086
run security list-keychains -d user -s "$keychain" $existing

# Kendinden imzalı sertifika "güvenilmez" görünür (-v ile listelenmez) ama
# codesign onunla imzalar; kimlik SHA-1 özetiyle seçilir.
identities=$(security find-identity -p codesigning "$keychain" 2>&1 || true)
echo "$identities"
identity=$(echo "$identities" | awk '/^ *[0-9]+\) [0-9A-F]+ / { print $2; exit }')
if [ -z "$identity" ]; then
  fail "Anahtar zincirinde kod imzalama kimliği bulunamadı:"$'\n'"$identities"
fi
echo "APPLE_SIGNING_IDENTITY=$identity" >> "$GITHUB_ENV"
echo "İmza kimliği: $identity"
