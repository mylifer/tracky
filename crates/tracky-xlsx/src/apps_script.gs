/**
 * Kum zaman çizelgesi köprüsü: Kum'un onaylanan kayıtlarını bu tablonun ilk sayfasına ekler.
 *
 * Kurulum: Uzantılar → Apps Script, bu kodu yapıştır, Dağıt → Yeni dağıtım → Web uygulaması
 * ("Şu kullanıcı olarak yürüt: Ben", "Erişimi olanlar: Herkes"). Çıkan /exec adresini Kum'a gir.
 * Anahtar (TOKEN) yalnızca Kum'dan gelen istekleri kabul etmek içindir; kimseyle paylaşma.
 *
 * Excel aktarımıyla aynı kurallar: sütunlar başlık satırındaki adlardan bulunur; önce o günün
 * önceden doldurulmuş boş satırı kullanılır, yoksa o günün son satırının altına satır eklenir
 * ve biçimi üstteki satırdan alınır. Aynı kayıt iki kez gönderilse de bir kez yazılır.
 * Son aktarımın satırları işaretlenir; Kum'dan "Geri al" denince silinir ya da boşaltılır.
 * Kum tablodaki satırları okur (list) ve tek tek değiştirir, kaldırır ya da geri ekler (update,
 * remove, insert): satır, beklenen eski içeriğiyle bulunur; tabloda değişmişse dokunulmaz.
 *
 * @OnlyCurrentDoc
 */
const TOKEN = "{{TOKEN}}";
const HEADER_ROW = 1;
const DONE_KEY = "kum_done";
const DONE_MAX = 500;
/** Son aktarımda yazılan satırın işareti: değeri "<kimlik>|<1: eklendi, 0: dolduruldu>". */
const ROW_KEY = "kum_row";
const DAY_FORMULA =
  '=SWITCH(WEEKDAY({c}),1,"Sunday",2,"Monday",3,"Tuesday",4,"Wednesday",5,"Thursday",6,"Friday",7,"Saturday")';

function doPost(e) {
  let out;
  try {
    const req = JSON.parse(e.postData.contents);
    if (req.token !== TOKEN) throw new Error("Anahtar uyuşmuyor: betiği Kum'dan yeniden kopyala.");
    const sheet = SpreadsheetApp.getActiveSpreadsheet().getSheets()[0];
    const writes = {
      append: () => append_(sheet, req.consultant || "", req.rows || []),
      undo: () => undo_(sheet, req.ids || []),
      update: () => update_(sheet, req.consultant || "", req.expect, req.row),
      remove: () => remove_(sheet, req.expect, req.id),
      insert: () => insert_(sheet, req.consultant || "", req.row),
    };
    if (req.action === "inspect") {
      out = inspect_(sheet);
    } else if (req.action === "list") {
      out = list_(sheet, req.from, req.to);
    } else if (writes[req.action]) {
      const lock = LockService.getDocumentLock();
      lock.waitLock(30000);
      try {
        out = writes[req.action]();
      } finally {
        lock.releaseLock();
      }
    } else {
      throw new Error("Bilinmeyen işlem: " + req.action);
    }
    out.ok = true;
    out.sheet = sheet.getName();
  } catch (err) {
    out = { ok: false, error: String((err && err.message) || err) };
  }
  return ContentService.createTextOutput(JSON.stringify(out)).setMimeType(ContentService.MimeType.JSON);
}

function doGet() {
  return ContentService.createTextOutput(JSON.stringify({ ok: true, kum: 1 })).setMimeType(
    ContentService.MimeType.JSON,
  );
}

function norm_(v) {
  return String(v).split(/\s+/).filter(Boolean).join(" ").toLowerCase();
}

function columns_(sheet) {
  const last = Math.max(sheet.getLastColumn(), 1);
  const headers = sheet.getRange(HEADER_ROW, 1, 1, last).getValues()[0].map(norm_);
  const find = (name, pred, optional) => {
    const i = headers.findIndex(pred);
    if (i < 0 && !optional) throw new Error('Başlık satırında "' + name + '" sütunu bulunamadı');
    return i < 0 ? null : i + 1;
  };
  return {
    last: last,
    date: find("Date", (h) => h === "date"),
    day: find("Day", (h) => h === "day", true),
    consultant: find("Consultant", (h) => h === "consultant", true),
    start: find("Started at", (h) => h.startsWith("started")),
    hours: find("Amount of Hours", (h) => h.includes("hours")),
    kind: find("Type", (h) => h === "type"),
    details: find("Details", (h) => h === "details"),
    party: find("Parties", (h) => h.startsWith("part")),
    division: find("Division", (h) => h.includes("division") || h === "project" || h === "proje"),
  };
}

/** Hücre değeri → "yyyy-MM-dd" (tarih değilse null). */
function iso_(v, tz) {
  if (Object.prototype.toString.call(v) === "[object Date]" && !isNaN(v)) return Utilities.formatDate(v, tz, "yyyy-MM-dd");
  if (typeof v === "number" && v > 0) {
    const d = new Date(Date.UTC(1899, 11, 30) + Math.floor(v) * 86400000);
    return Utilities.formatDate(d, "UTC", "yyyy-MM-dd");
  }
  return null;
}

/** "yyyy-MM-dd" → tablo seri numarası (1899-12-30'dan beri gün). */
function serial_(iso) {
  const p = iso.split("-").map(Number);
  return (Date.UTC(p[0], p[1] - 1, p[2]) - Date.UTC(1899, 11, 30)) / 86400000;
}

/**
 * Metin hücresi: baştaki "'" Sheets'e değerin metin olduğunu söyler (hücrede görünmez, okurken
 * gelmez). "=", "+", "-", "@" ile başlayan metin formül, "1/2", "10:00", "12%" tarih, saat ya da
 * sayı olmaz. Hücre biçimine dokunulmaz: düz metin biçimli hücrede "'" görünür kalırdı.
 */
function setText_(range, v) {
  const s = String(v);
  range.setValue(s === "" ? "" : "'" + s);
}

function blank_(v) {
  return String(v).trim() === "";
}

function inspect_(sheet) {
  const cols = columns_(sheet);
  const n = Math.max(sheet.getLastRow() - HEADER_ROW, 0);
  const values = n ? sheet.getRange(HEADER_ROW + 1, 1, n, cols.last).getValues() : [];
  const ranked = (col) => {
    const counts = {};
    for (const row of values) {
      const v = String(row[col - 1]).trim();
      if (v) counts[v] = (counts[v] || 0) + 1;
    }
    return Object.keys(counts).sort((a, b) => counts[b] - counts[a] || (a < b ? -1 : a > b ? 1 : 0));
  };
  // Birim sütununun başlığından firma: "Togg Division" → "Togg".
  const company = [];
  for (const w of String(sheet.getRange(HEADER_ROW, cols.division).getValue()).split(/\s+/).filter(Boolean)) {
    if (w.toLowerCase() === "division") break;
    company.push(w);
  }
  return {
    company: company.join(" ") || null,
    consultant: cols.consultant ? ranked(cols.consultant)[0] || null : null,
    parties: ranked(cols.party),
    divisions: ranked(cols.division),
    details: ranked(cols.details),
  };
}

function append_(sheet, consultant, rows) {
  const props = PropertiesService.getDocumentProperties();
  const done = JSON.parse(props.getProperty(DONE_KEY) || "[]");
  const seen = new Set(done);
  const tz = sheet.getParent().getSpreadsheetTimeZone();
  const cols = columns_(sheet);
  const todo = rows
    .filter((r) => !seen.has(String(r.id).slice(0, 13)))
    .sort((a, b) => (a.date + a.start < b.date + b.start ? -1 : a.date + a.start > b.date + b.start ? 1 : 0));
  // Yalnızca son aktarım geri alınabilir: önceki işaretler silinir (meta veri sınırlı).
  for (const m of sheet.createDeveloperMetadataFinder().withKey(ROW_KEY).find()) m.remove();
  let filled = 0;
  let inserted = 0;
  for (const row of todo) {
    const { target, fresh } = place_(sheet, cols, row.date, row.start, tz);
    if (fresh) inserted++;
    else filled++;
    put_(sheet, target, cols, consultant, row);
    if (cols.day) day_(sheet, target, cols, row.date);
    sheet
      .getRange(target + ":" + target)
      .addDeveloperMetadata(ROW_KEY, String(row.id).slice(0, 13) + "|" + (fresh ? 1 : 0));
    // Her satırdan sonra: betik yarıda kesilirse yeniden denemede yazılanlar atlansın.
    SpreadsheetApp.flush();
    done.push(String(row.id).slice(0, 13));
    props.setProperty(DONE_KEY, JSON.stringify(done.slice(-DONE_MAX)));
  }
  return { filled: filled, inserted: inserted, skipped: rows.length - todo.length };
}

/**
 * Son aktarımda yazılan `ids` satırlarını geri alır: eklenen satır silinir, önceden var olan
 * boş satırın yazılan hücreleri boşaltılır. Kayıtlar yeniden gönderilebilsin diye unutulur.
 */
function undo_(sheet, ids) {
  const want = new Set(ids.map((id) => String(id).slice(0, 13)));
  const cols = columns_(sheet);
  const hits = [];
  for (const m of sheet.createDeveloperMetadataFinder().withKey(ROW_KEY).find()) {
    const [id, fresh] = m.getValue().split("|");
    if (want.has(id)) hits.push({ id: id, fresh: fresh === "1", row: m.getLocation().getRow().getRow(), meta: m });
  }
  // Alttan yukarı: silinen satır üsttekilerin yerini kaydırmasın.
  hits.sort((a, b) => b.row - a.row);
  let removed = 0;
  let cleared = 0;
  for (const h of hits) {
    if (h.fresh) {
      sheet.deleteRow(h.row);
      removed++;
    } else {
      for (const c of [cols.start, cols.hours, cols.kind, cols.details, cols.party, cols.division]) {
        sheet.getRange(h.row, c).clearContent();
      }
      h.meta.remove();
      cleared++;
    }
  }
  const props = PropertiesService.getDocumentProperties();
  const gone = new Set(hits.map((h) => h.id));
  const done = JSON.parse(props.getProperty(DONE_KEY) || "[]").filter((id) => !gone.has(id));
  props.setProperty(DONE_KEY, JSON.stringify(done));
  return { removed: removed, cleared: cleared, missing: want.size - gone.size, undone: Array.from(gone) };
}

/** Tablonun kayıt satırları: değerler, başlangıç için görünen metin ("09:30") ve satır numarası. */
function rows_(sheet, cols) {
  const tz = sheet.getParent().getSpreadsheetTimeZone();
  const n = Math.max(sheet.getLastRow() - HEADER_ROW, 0);
  if (!n) return [];
  const range = sheet.getRange(HEADER_ROW + 1, 1, n, cols.last);
  const values = range.getValues();
  const display = range.getDisplayValues();
  return values.map((v, i) => read_(v, display[i], HEADER_ROW + 1 + i, cols, tz));
}

/** Satır → Kum'un okuduğu kayıt; tarihsiz ya da boş (önceden doldurulmuş gün satırı) ise null. */
function read_(v, d, r, cols, tz) {
  const date = iso_(v[cols.date - 1], tz);
  if (!date) return null;
  const text = (c) => (c ? String(v[c - 1]).trim() : "");
  // Görünen metin ("09:30"); biçimsiz hücrede gün kesri ya da tarih-saat değeri.
  const raw = v[cols.start - 1];
  let hm = /(\d{1,2}):(\d{2})/.exec(String(d[cols.start - 1]));
  if (!hm && typeof raw === "number" && raw >= 0) {
    const min = Math.round((raw % 1) * 1440) % 1440;
    hm = [null, String(Math.floor(min / 60)), ("0" + (min % 60)).slice(-2)];
  } else if (!hm && Object.prototype.toString.call(raw) === "[object Date]" && !isNaN(raw)) {
    hm = /(\d{2}):(\d{2})/.exec(Utilities.formatDate(raw, tz, "HH:mm"));
  }
  const start = hm ? ("0" + hm[1]).slice(-2) + ":" + hm[2] + ":00" : null;
  let hours = v[cols.hours - 1];
  if (typeof hours !== "number") {
    const h = parseFloat(String(hours).replace(",", "."));
    hours = isNaN(h) ? null : h;
  }
  const out = {
    row: r,
    date: date,
    start: start,
    hours: hours,
    kind: text(cols.kind),
    details: text(cols.details),
    party: text(cols.party),
    division: text(cols.division),
    consultant: text(cols.consultant),
  };
  return out.kind || out.details || out.hours !== null ? out : null;
}

/**
 * İki kaydın içeriği aynı mı (satır numarası hariç). Danışman ikisinde de yazılıysa aynı olmalı:
 * ortak tabloda iş arkadaşının aynı içerikli satırı değiştirilmesin.
 */
function same_(a, b) {
  const t = (s) => String(s == null ? "" : s).trim();
  const who = (s) => t(s).toLowerCase();
  const minute = (s) => (s ? String(s).slice(0, 5) : null);
  const hours = a.hours == null || b.hours == null ? a.hours == b.hours : Math.abs(a.hours - b.hours) < 1e-6;
  return (
    a.date === b.date &&
    minute(a.start) === minute(b.start) &&
    hours &&
    t(a.kind) === t(b.kind) &&
    t(a.details) === t(b.details) &&
    t(a.party) === t(b.party) &&
    t(a.division).toLowerCase() === t(b.division).toLowerCase() &&
    (!who(a.consultant) || !who(b.consultant) || who(a.consultant) === who(b.consultant))
  );
}

/** Beklenen kaydın bugünkü satırı: ipucundaki satır hâlâ aynıysa o, değilse içeriği aynı tek satır. */
function locate_(rows, expect) {
  const at = (i) => rows[i] && same_(rows[i], expect);
  const hint = expect.row - HEADER_ROW - 1;
  if (hint >= 0 && hint < rows.length && at(hint)) return expect.row;
  const found = [];
  for (let i = 0; i < rows.length; i++) if (at(i)) found.push(HEADER_ROW + 1 + i);
  if (found.length !== 1) throw new Error("Satır değişmiş ya da silinmiş; sayfayı yenileyip tekrar dene.");
  return found[0];
}

function list_(sheet, from, to) {
  const rows = rows_(sheet, columns_(sheet)).filter((r) => r && r.date >= from && r.date <= to);
  return { rows: rows };
}

/** Sıralama anahtarı: gün ve saat; saatsiz kayıt günün başında sayılır. */
function key_(date, start) {
  return date + " " + (start ? String(start).slice(0, 5) : "00:00");
}

/** Başlık altındaki satırlar, yerleştirme için: tarih ve (yalnızca kayıtta) sıralama anahtarı. */
function lines_(sheet, cols, tz) {
  const n = Math.max(sheet.getLastRow() - HEADER_ROW, 0);
  if (!n) return [];
  const range = sheet.getRange(HEADER_ROW + 1, 1, n, cols.last);
  const values = range.getValues();
  const display = range.getDisplayValues();
  return values.map((v, i) => {
    const e = read_(v, display[i], HEADER_ROW + 1 + i, cols, tz);
    return { date: iso_(v[cols.date - 1], tz), key: e ? key_(e.date, e.start) : null };
  });
}

/**
 * Yeni kaydın satırı (Kum'daki `slot` ile aynı): tablo gün ve saate göre sıralı kalır. Kendisinden
 * önce gelen son kaydın altındaki aralıkta (bir sonraki kayda kadar) o günün boş satırı varsa o
 * doldurulur; yoksa aralıkta tarihi önce gelen boş gün satırlarının altına satır eklenir.
 */
function slot_(lines, date, start) {
  const k = key_(date, start);
  let prev = -1;
  for (let i = 0; i < lines.length; i++) if (lines[i].key !== null && lines[i].key <= k) prev = i;
  let to = lines.length;
  for (let i = prev + 1; i < lines.length; i++) {
    if (lines[i].key !== null) {
      to = i;
      break;
    }
  }
  for (let i = prev + 1; i < to; i++) {
    if (lines[i].key === null && lines[i].date === date) return { target: HEADER_ROW + 1 + i, fresh: false };
  }
  let after = prev;
  for (let i = to - 1; i > prev; i--) {
    if (lines[i].key === null && lines[i].date && lines[i].date <= date) {
      after = i;
      break;
    }
  }
  return { target: HEADER_ROW + 2 + after, fresh: true };
}

/** `i` kaydı yeni gün ve saatiyle yerinde kalabilir mi: üstündeki kayıt önce, altındaki sonra. */
function inOrder_(lines, i, date, start) {
  const k = key_(date, start);
  for (let j = i - 1; j >= 0; j--) if (lines[j].key !== null) {
    if (lines[j].key > k) return false;
    break;
  }
  for (let j = i + 1; j < lines.length; j++) if (lines[j].key !== null) return lines[j].key >= k;
  return true;
}

/**
 * `date` günü `start` saatli kaydın satırı (`slot_`); eklenen satır biçimini üstteki kayıt satırından
 * (başlığın hemen altındaysa alttakinden) alır.
 */
function place_(sheet, cols, date, start, tz) {
  const s = slot_(lines_(sheet, cols, tz), date, start);
  if (!s.fresh) return s;
  sheet.insertRowBefore(s.target);
  const from = s.target - 1 > HEADER_ROW ? s.target - 1 : s.target + 1;
  if (from <= sheet.getLastRow()) {
    sheet
      .getRange(from, 1, 1, cols.last)
      .copyTo(sheet.getRange(s.target, 1, 1, cols.last), SpreadsheetApp.CopyPasteType.PASTE_FORMAT, false);
  }
  return s;
}

/**
 * `r` satırındaki `date` günlü kaydı kaldırır: günün başka satırı varsa satır silinir, yoksa gün
 * satırı kalır, kayıt hücreleri boşaltılır. Satır silindiyse true.
 */
function vacate_(sheet, cols, r, date, tz) {
  const n = Math.max(sheet.getLastRow() - HEADER_ROW, 0);
  const dates = sheet.getRange(HEADER_ROW + 1, cols.date, n, 1).getValues().map((v) => iso_(v[0], tz));
  const others = dates.some((d, i) => d === date && HEADER_ROW + 1 + i !== r);
  if (others) {
    sheet.deleteRow(r);
  } else {
    for (const c of [cols.start, cols.hours, cols.kind, cols.details, cols.party, cols.division]) {
      sheet.getRange(r, c).clearContent();
    }
    for (const m of sheet.createDeveloperMetadataFinder().withKey(ROW_KEY).find()) {
      if (m.getLocation().getRow().getRow() === r) m.remove();
    }
  }
  return others;
}

/** Kaydı yeni değerleriyle yazar; tarih değiştiyse ya da yeni saatiyle sıra bozulacaksa eski yerinden kaldırılıp yeniden yerleşir. */
function update_(sheet, consultant, expect, row) {
  const cols = columns_(sheet);
  const tz = sheet.getParent().getSpreadsheetTimeZone();
  let r = locate_(rows_(sheet, cols), expect);
  if (row.date !== expect.date || !inOrder_(lines_(sheet, cols, tz), r - HEADER_ROW - 1, row.date, row.start)) {
    vacate_(sheet, cols, r, expect.date, tz);
    r = place_(sheet, cols, row.date, row.start, tz).target;
  }
  put_(sheet, r, cols, consultant, row);
  if (cols.day) day_(sheet, r, cols, row.date);
  return { row: r };
}

/** Tek kaydı gününe ekler (Kum'dan silmenin geri alınması); son aktarımın işaretlerine dokunmaz. */
function insert_(sheet, consultant, row) {
  const cols = columns_(sheet);
  const r = place_(sheet, cols, row.date, row.start, sheet.getParent().getSpreadsheetTimeZone()).target;
  put_(sheet, r, cols, consultant, row);
  if (cols.day) day_(sheet, r, cols, row.date);
  return { row: r };
}

/** Kaydı kaldırır: günün başka satırı varsa satır silinir, yoksa gün satırı kalır, kayıt hücreleri boşaltılır. */
function remove_(sheet, expect, id) {
  const cols = columns_(sheet);
  const tz = sheet.getParent().getSpreadsheetTimeZone();
  const r = locate_(rows_(sheet, cols), expect);
  const removed = vacate_(sheet, cols, r, expect.date, tz);
  if (id) {
    const props = PropertiesService.getDocumentProperties();
    const key = String(id).slice(0, 13);
    const done = JSON.parse(props.getProperty(DONE_KEY) || "[]").filter((x) => x !== key);
    props.setProperty(DONE_KEY, JSON.stringify(done));
  }
  return { removed: removed };
}

/**
 * Day hücresi tablonun kendi yöntemiyle doldurulur: sütunda ARRAYFORMULA varsa hiç
 * dokunulmaz (yazılan her değer dizi formülünü bozar); yoksa üstteki dolu Day hücresi
 * örnek alınır (formülse göreli olarak kopyalanır, değerse aynı türde değer yazılır).
 */
function day_(sheet, r, cols, iso) {
  const cell = sheet.getRange(r, cols.day);
  if (cell.getFormula() || !blank_(cell.getValue())) return;
  const col = sheet.getRange(HEADER_ROW, cols.day, sheet.getLastRow() - HEADER_ROW + 1, 1);
  const formulas = col.getFormulas().map((f) => f[0]);
  if (formulas.some((f) => /arrayformula/i.test(f))) return;
  const values = col.getValues().map((v) => v[0]);
  for (let i = r - HEADER_ROW - 1; i > 0; i--) {
    if (formulas[i]) {
      sheet.getRange(HEADER_ROW + i, cols.day).copyTo(cell, SpreadsheetApp.CopyPasteType.PASTE_FORMULA, false);
      return;
    }
    if (!blank_(values[i])) {
      if (Object.prototype.toString.call(values[i]) === "[object Date]") cell.setValue(serial_(iso));
      else cell.setValue(Utilities.formatDate(new Date(iso + "T12:00:00Z"), "UTC", "EEEE"));
      return;
    }
  }
  cell.setFormula(DAY_FORMULA.replace("{c}", sheet.getRange(r, cols.date).getA1Notation()));
}

function put_(sheet, r, cols, consultant, row) {
  sheet.getRange(r, cols.date).setValue(serial_(row.date)).setNumberFormat("d/m/yy");
  if (cols.consultant && consultant.trim()) setText_(sheet.getRange(r, cols.consultant), consultant.trim());
  const [hh, mm] = row.start.split(":").map(Number);
  sheet
    .getRange(r, cols.start)
    .setValue((hh * 60 + mm) / 1440)
    .setNumberFormat("hh:mm");
  // Genel biçim: 1, 0,5, 0,25 (gereksiz sıfırlar olmadan).
  sheet.getRange(r, cols.hours).setValue(row.hours).setNumberFormat("General");
  setText_(sheet.getRange(r, cols.kind), row.kind);
  setText_(sheet.getRange(r, cols.details), row.details);
  setText_(sheet.getRange(r, cols.party), row.party);
  setText_(sheet.getRange(r, cols.division), row.division);
}
