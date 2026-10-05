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
 *
 * @OnlyCurrentDoc
 */
const TOKEN = "{{TOKEN}}";
const HEADER_ROW = 1;
const DONE_KEY = "kum_done";
const DONE_MAX = 500;
const DAY_FORMULA =
  '=SWITCH(WEEKDAY({c}),1,"Sunday",2,"Monday",3,"Tuesday",4,"Wednesday",5,"Thursday",6,"Friday",7,"Saturday")';

function doPost(e) {
  let out;
  try {
    const req = JSON.parse(e.postData.contents);
    if (req.token !== TOKEN) throw new Error("Anahtar uyuşmuyor: betiği Kum'dan yeniden kopyala.");
    const sheet = SpreadsheetApp.getActiveSpreadsheet().getSheets()[0];
    if (req.action === "inspect") {
      out = inspect_(sheet);
    } else if (req.action === "append") {
      const lock = LockService.getDocumentLock();
      lock.waitLock(30000);
      try {
        out = append_(sheet, req.consultant || "", req.rows || []);
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

/** "=" ya da "+" ile başlayan metin formül sayılmasın. */
function text_(v) {
  const s = String(v);
  return /^[=+]/.test(s) ? "'" + s : s;
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
  let filled = 0;
  let inserted = 0;
  for (const row of todo) {
    const n = Math.max(sheet.getLastRow() - HEADER_ROW, 0);
    const values = n ? sheet.getRange(HEADER_ROW + 1, 1, n, cols.last).getValues() : [];
    const dates = values.map((r) => iso_(r[cols.date - 1], tz));
    let target = -1;
    for (let i = 0; i < values.length; i++) {
      if (dates[i] === row.date && blank_(values[i][cols.details - 1]) && blank_(values[i][cols.kind - 1])) {
        target = HEADER_ROW + 1 + i;
        break;
      }
    }
    if (target > 0) {
      filled++;
    } else {
      // O günün son satırının, yoksa daha önceki son tarihin altına.
      let after = HEADER_ROW;
      for (let i = 0; i < dates.length; i++) if (dates[i] && dates[i] <= row.date) after = HEADER_ROW + 1 + i;
      sheet.insertRowAfter(after);
      target = after + 1;
      if (after > HEADER_ROW) {
        sheet
          .getRange(after, 1, 1, cols.last)
          .copyTo(sheet.getRange(target, 1, 1, cols.last), SpreadsheetApp.CopyPasteType.PASTE_FORMAT, false);
      }
      inserted++;
    }
    put_(sheet, target, cols, consultant, row);
    if (cols.day) day_(sheet, target, cols);
    // Her satırdan sonra: betik yarıda kesilirse yeniden denemede yazılanlar atlansın.
    SpreadsheetApp.flush();
    done.push(String(row.id).slice(0, 13));
    props.setProperty(DONE_KEY, JSON.stringify(done.slice(-DONE_MAX)));
  }
  return { filled: filled, inserted: inserted, skipped: rows.length - todo.length };
}

/**
 * Day hücresi tablonun kendi yöntemiyle doldurulur: sütunda ARRAYFORMULA varsa hiç
 * dokunulmaz (yazılan her değer dizi formülünü bozar); yoksa üstteki dolu Day hücresi
 * örnek alınır (formülse göreli olarak kopyalanır, değerse aynı türde değer yazılır).
 */
function day_(sheet, r, cols) {
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
      const date = sheet.getRange(r, cols.date).getValue();
      if (Object.prototype.toString.call(values[i]) === "[object Date]") cell.setValue(date);
      else cell.setValue(Utilities.formatDate(date, sheet.getParent().getSpreadsheetTimeZone(), "EEEE"));
      return;
    }
  }
  cell.setFormula(DAY_FORMULA.replace("{c}", sheet.getRange(r, cols.date).getA1Notation()));
}

function put_(sheet, r, cols, consultant, row) {
  sheet.getRange(r, cols.date).setValue(serial_(row.date)).setNumberFormat("d/m/yy");
  if (cols.consultant && consultant.trim()) sheet.getRange(r, cols.consultant).setValue(text_(consultant.trim()));
  const [hh, mm] = row.start.split(":").map(Number);
  sheet
    .getRange(r, cols.start)
    .setValue((hh * 60 + mm) / 1440)
    .setNumberFormat("hh:mm");
  // Genel biçim: 1, 0,5, 0,25 (gereksiz sıfırlar olmadan).
  sheet.getRange(r, cols.hours).setValue(row.hours).setNumberFormat("General");
  sheet.getRange(r, cols.kind).setValue(text_(row.kind));
  sheet.getRange(r, cols.details).setValue(text_(row.details));
  sheet.getRange(r, cols.party).setValue(text_(row.party));
  sheet.getRange(r, cols.division).setValue(text_(row.division));
}
