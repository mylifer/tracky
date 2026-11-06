# Gün planı: davranış kuralları

Bu belge gün planının (Gün görünümündeki "Plan" sütunu) nasıl davrandığını tanımlar. Kurallar
kullanıcıyla tek tek kararlaştırıldı; kod bu belgeye uyar, belgede olmayan davranış eklenmez.
Bir kural değişecekse önce bu belge değişir.

## Temel ilke

- **Ham kayıt** (pencereler, uygulamalar, takvim) kanıttır. Plan ona yazmaz; o da planı değiştirmez.
- **Plan blokları** kullanıcınındır. Bir blok yalnızca kullanıcı onu değiştirince değişir; hiçbir
  işlem başka bir bloğu kendiliğinden değiştirmez, bölmez, büyütmez ya da birleştirmez.
- İki tür blok vardır:
  - **Öneri** (kesik çizgili): sistemin önerisidir; onaylanmadıkça hiçbir yere yazılmaz.
  - **Onaylı** (dolu): kullanıcının kabul ettiği bloktur; zaman çizelgesine gider.

## Kurallar

1. **Takvim etkinliği.** Takvimdeki etkinlik, Plan sütununda tam saatinde ve tam süresi kadar
   kesik çizgili bir öneri olarak görünür. Kullanıcı onu küçültüp büyütebilir. Onaylanınca
   zaman çizelgesine gider; onaylanmadıkça gitmez.
2. **Etkinlik sırasında bilgisayar işi.** Etkinliğin saatinde bilgisayarda başka iş olsa da
   o saat etkinliğindir. Bilgisayar işi ayrı öneri açmaz; yalnızca Pencereler sütununda kanıt
   olarak görünür.
3. **Etkinlik olmayan saatler.** Bilgisayar kaydından kesik çizgili öneriler gelir.
   - **3a.** Öneriler yalnızca "Öner" düğmesine basınca hazırlanır. Hazırlayan, kullanıcının
     geçmişte onayladığı bloklardan (hangi pencereler hangi projeye ve hangi ada gitti)
     bilgisayarda öğrenen bir modeldir. İnternete bağlanmaz, ücret yoktur.
   - **3b.** Öğrenecek yeterli geçmiş yoksa basit öneriler gelir: proje ve baskın pencereye
     göre, uzun boşlukta bölünen bloklar.
   - **3c.** Pencereler sütununda bir pencere ya da aralık projeye atanınca, o saat için Plan'da
     o projeyle kesik öneri belirir ("Öner"e basmadan). Aynı projede peş peşe atananlar tek
     öneri olur. Onaylı blokların ve etkinliklerin saatine öneri girmez; "projesiz" atama o
     saatteki öneriyi kaldırır.
4. **Boşluk.** Kullanıcının küçültme ya da silmeyle açtığı boşluk boş kalır. Sistem oraya ne
   komşu bloğu ne de öneriyi koyar.
5. **Büyütme.** Kenarı sürüklenen blok yalnızca kendisi değişir. Yanındaki bloğa değince
   onun kenarında durur.
6. **Zaman çizelgesi.** Onaylı her blok, kendi saat aralığı ve adıyla tek satırdır. Blok
   değişince satırı da değişir, blok silinince satır kalkar. Aynı projede art arda gelen
   bloklar ayrı satırlardır.
7. **Çizme.** Sütunda sürükleyerek yeni blok çizilir. Çizim yalnızca boş alana yapılır;
   onaylı bir bloğun kenarında durur, onu değiştirmez.
8. **Bölme ve birleştirme.** Yalnızca bloğun menüsünden yapılır: "Böl" ve
   "Üsttekiyle / Alttakiyle birleştir". Çift tıklama gibi kazara olabilecek kısayol yoktur.
9. **Raporlar.** Hafta, Ay ve proje süreleri bilgisayar kaydından hesaplanır. Plan ham kayda
   yazmaz; geçmiş aylar asla değişmez.
10. **Eski görünüm.** 1 Kasım 2026 ve sonrası yalnızca Plan ile açılır. Önceki günler eski
    "Oturumlar" görünümünde, hiçbir şeyleri değişmeden durur. Görünüm seçme menüsü kalkar.

## Ayrıntılar

Kurallarla birlikte onaylandı.

- **V1.** Kenarlar ve çizim 5 dakikaya oturur. Bir blok 5 dakikadan kısa olamaz.
- **V2.** Öneri onaylama: öneriye tıklayıp "Onayla", ya da başlıktaki "Hepsini onayla".
  Bir öneriye proje ya da ad vermek, kenarını sürüklemek de onu onaylar.
- **V3.** Silinen öneri, "Öner"e yeniden basılana dek geri gelmez. Silinen takvim etkinliği
  önerisi o gün için bir daha gelmez.
- **V4.** Projesi olmayan onaylı blok zaman çizelgesine gitmez (projesiz iş). Proje verilince gider.
- **V5.** Bloğun adı zaman çizelgesindeki açıklamadır. Ad yazarken önceki açıklamalar ve
  bloğun pencere başlıkları önerilir.
- **V6.** Onaylı bloklar bilgisayarlar arasında eşitlenir (Supabase `plan_blocks`). "Öner" ile
  hazırlanan öneriler o bilgisayarda kalır; takvim etkinliği önerileri her bilgisayarda görünür.
- **V7.** Etkinlik önerisinin projesi, toplantının proje atamasından gelir; yoksa projesizdir.
