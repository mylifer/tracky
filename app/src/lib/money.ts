/**
 * Yapay zekâ isteklerinin yaklaşık tutarı: dolar, Türkçe biçimde. Bir sentin altındaki tutarlar
 * "< $0,01" olur (kuruşluk istekler sıfır görünmesin); büyük tutarlarda iki basamak.
 */
const usd = new Intl.NumberFormat("tr-TR", { minimumFractionDigits: 2, maximumFractionDigits: 2 });

export function formatUsd(value: number): string {
  if (value <= 0) return "$0,00";
  if (value < 0.01) return "< $0,01";
  return `$${usd.format(value)}`;
}
