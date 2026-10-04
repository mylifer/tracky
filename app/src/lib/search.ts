/** Aramada karşılaştırma biçimi: küçük harf, Türkçe ı/i ve aksanlar sadeleşir ("Müşteri" ~ "musteri"). */
export function fold(s: string): string {
  return s.toLocaleLowerCase("tr").replace(/ı/g, "i").normalize("NFD").replace(/[̀-ͯ]/g, "");
}
