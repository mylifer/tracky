#!/usr/bin/env bash
# Apple notarization'ı tek komutla kurar: Developer ID sertifikasını denetler, Apple bilgilerini
# Apple'a sorarak doğrular ve sürüm iş akışının beklediği beş secret'ı GitHub'a yazar.
#
# Önce (Apple hesabına giriş gerektirdiği için elle):
#   1. Xcode → Settings → Accounts → hesap → Manage Certificates → + → Developer ID Application
#   2. Anahtar Zinciri Erişimi'nde "Developer ID Application: …" satırına sağ tık → Dışa Aktar
#      → .p12 (bir parola ver)
#   3. account.apple.com → Giriş ve Güvenlik → Uygulamaya Özel Parolalar → yeni parola
#
# Kullanım: scripts/macos-notarize-setup.sh [~/Desktop/kum.p12]
# Gerekenler: macOS, Xcode komut satırı araçları (xcrun notarytool), giriş yapılmış `gh`.
#
# Parolalar ekranda görünmez, dosyaya ya da günlüğe yazılmaz; yalnızca GitHub'a (şifreli
# secret olarak) ve doğrulama için Apple'a gider.
set -euo pipefail

REPO="${KUM_REPO:-mylifer/tracky}"

die() {
  echo "Hata: $*" >&2
  exit 1
}

[ "$(uname)" = Darwin ] || die "bu betik macOS'ta çalışır."
command -v gh > /dev/null || die "GitHub CLI yok. Kur: brew install gh, sonra: gh auth login"
gh auth status > /dev/null 2>&1 || die "GitHub'a giriş yapılmamış. Çalıştır: gh auth login"
xcrun --find notarytool > /dev/null 2>&1 || die "notarytool yok. Çalıştır: xcode-select --install"

p12="${1:-}"
if [ -z "$p12" ]; then
  read -r -p "Dışa aktardığın .p12 dosyası (sürükleyip bırakabilirsin): " p12
fi
# Sürükle-bırakta gelen tırnak ve kaçış karakterleri.
p12="${p12%\"}"
p12="${p12#\"}"
p12="${p12//\\ / }"
p12="${p12/#\~/$HOME}"
[ -f "$p12" ] || die "dosya bulunamadı: $p12"

read -r -s -p ".p12 parolası: " p12_pw
echo

# macOS'taki LibreSSL eski biçimi doğrudan okur; Homebrew OpenSSL 3 için -legacy gerekir.
pkcs12() {
  openssl pkcs12 -in "$p12" -passin "pass:$p12_pw" "$@" 2> /dev/null \
    || openssl pkcs12 -legacy -in "$p12" -passin "pass:$p12_pw" "$@" 2> /dev/null
}

certs=$(pkcs12 -nokeys -clcerts) || die ".p12 açılamadı: parola yanlış ya da dosya bozuk."
pkcs12 -nocerts -nodes | grep -q "PRIVATE KEY" \
  || die ".p12'de özel anahtar yok. Anahtar Zinciri'nde sertifikanın kendisini (altındaki anahtarla) dışa aktar."
count=$(printf '%s\n' "$certs" | grep -c "BEGIN CERTIFICATE" || true)
[ "$count" = 1 ] \
  || die ".p12'de $count sertifika var; yalnızca Developer ID Application sertifikasını dışa aktar."
subject=$(printf '%s\n' "$certs" | openssl x509 -noout -subject -nameopt multiline,-esc_msb)
printf '%s\n' "$subject" | grep -q "Developer ID Application" \
  || die "bu bir Developer ID Application sertifikası değil:"$'\n'"$subject"
team=$(printf '%s\n' "$subject" | sed -n 's/^ *organizationalUnitName *= *//p' | head -1)
[[ "$team" =~ ^[A-Z0-9]{10}$ ]] || die "sertifikadan Team ID okunamadı:"$'\n'"$subject"
name=$(printf '%s\n' "$subject" | sed -n 's/^ *commonName *= *//p' | head -1)
echo "Sertifika: $name"
echo "Team ID:   $team"

read -r -p "Apple hesabının e-postası: " apple_id
[ -n "$apple_id" ] || die "e-posta boş."
read -r -s -p "Uygulamaya özel parola (xxxx-xxxx-xxxx-xxxx): " apple_pw
echo
[ -n "$apple_pw" ] || die "parola boş."

echo "Apple bilgileri doğrulanıyor…"
if ! out=$(xcrun notarytool history --apple-id "$apple_id" --password "$apple_pw" --team-id "$team" 2>&1); then
  die "Apple bilgileri kabul etmedi. E-postayı ve uygulamaya özel parolayı kontrol et (hesap parolası çalışmaz):"$'\n'"$out"
fi
echo "Apple bilgileri doğru."

echo "GitHub secret'ları yazılıyor ($REPO)…"
set_secret() {
  printf '%s' "$2" | gh secret set "$1" --repo "$REPO" > /dev/null || die "$1 yazılamadı (depoda yönetici yetkin var mı?)."
  echo "  ✓ $1"
}
set_secret MACOS_CERTIFICATE "$(base64 -i "$p12" | tr -d '\n')"
set_secret MACOS_CERTIFICATE_PASSWORD "$p12_pw"
set_secret APPLE_ID "$apple_id"
set_secret APPLE_PASSWORD "$apple_pw"
set_secret APPLE_TEAM_ID "$team"

echo
echo "Tamam. Bir sonraki sürüm notarize edilmiş olarak çıkar."
echo "Güncellemeden sonra Erişilebilirlik iznini bir kez yeniden ver (imza sertifikası değişti)."
