#!/usr/bin/env bash
# macOS paketleri için kendinden imzalı kod imzalama sertifikası üretir.
#
# Neden: ad-hoc imzalı uygulamanın kimliği her derlemede değişir; macOS
# Erişilebilirlik iznini bu kimliğe bağladığı için her güncellemede izin
# yeniden istenir. Sabit bir sertifikayla imzalanınca izin korunur.
# (Gatekeeper uyarısı ilk kurulumda yine çıkar; onun için Apple Developer ID gerekir.)
#
# Kullanım: scripts/macos-sign-cert.sh [çıktı klasörü]   (varsayılan ~/.kum-signing)
# Çıktıdaki iki değeri GitHub'da secret olarak ekleyin:
#   MACOS_CERTIFICATE           = kum-codesign.p12.base64 dosyasının içeriği
#   MACOS_CERTIFICATE_PASSWORD  = kum-codesign.password dosyasının içeriği
# Klasörü yedekleyin: sertifika değişirse kullanıcılar izni bir kez daha verir.
set -euo pipefail
# Özel anahtar yazılırken bile yalnızca sahibi okuyabilsin.
umask 077

out="${1:-$HOME/.kum-signing}"
name="Kum Self-Signed"
mkdir -p "$out"
cd "$out"
if [ -e kum-codesign.p12 ]; then
  echo "$out/kum-codesign.p12 zaten var; üzerine yazılmadı." >&2
  exit 1
fi

cat > kum-codesign.cnf <<EOF
[req]
distinguished_name = dn
prompt = no
x509_extensions = ext
[dn]
CN = $name
O = Kum
[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
subjectKeyIdentifier = hash
EOF

openssl req -x509 -newkey rsa:2048 -nodes -sha256 -days 7300 \
  -config kum-codesign.cnf -keyout kum-codesign.key -out kum-codesign.crt
# Satır sonu olmadan: secret'a yapıştırılan değerle birebir aynı olsun (Windows CRLF dahil).
openssl rand -hex 24 | tr -d '\r\n' > kum-codesign.password
# macOS `security import` eski PKCS#12 şifrelemesini en güvenilir biçimde okur.
openssl pkcs12 -export -name "$name" -inkey kum-codesign.key -in kum-codesign.crt \
  -keypbe PBE-SHA1-3DES -certpbe PBE-SHA1-3DES -macalg sha1 \
  -passout file:kum-codesign.password -out kum-codesign.p12
openssl base64 -A -in kum-codesign.p12 > kum-codesign.p12.base64
chmod 600 kum-codesign.key kum-codesign.p12 kum-codesign.p12.base64 kum-codesign.password

echo "Sertifika: $(openssl x509 -in kum-codesign.crt -noout -subject -enddate)"
echo "Dosyalar: $out"
