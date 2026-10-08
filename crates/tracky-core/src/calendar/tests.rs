use super::*;

fn n(s: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
}

fn utc(s: &str) -> DateTime<Utc> {
    n(s).and_utc()
}

fn occ(rule: &str, start: &str, end: &str) -> Vec<String> {
    expand(rule, n(start), n(end), |u| u.naive_utc())
        .into_iter()
        .map(|d| d.format("%Y-%m-%d %a").to_string())
        .collect()
}

#[test]
fn expands_common_outlook_rules() {
    // Her hafta Pazartesi ve Çarşamba, 4 kez.
    assert_eq!(
        occ(
            "FREQ=WEEKLY;COUNT=4;BYDAY=MO,WE;WKST=SU",
            "2026-10-05 10:00",
            "2027-01-01 00:00"
        ),
        [
            "2026-10-05 Mon",
            "2026-10-07 Wed",
            "2026-10-12 Mon",
            "2026-10-14 Wed"
        ]
    );
    // İki haftada bir Salı, UNTIL'e kadar (dahil).
    assert_eq!(
        occ(
            "FREQ=WEEKLY;INTERVAL=2;BYDAY=TU;UNTIL=20261103T070000Z",
            "2026-10-06 10:00",
            "2027-01-01 00:00"
        ),
        ["2026-10-06 Tue", "2026-10-20 Tue"]
    );
    // Her ayın son Cuması.
    assert_eq!(
        occ(
            "FREQ=MONTHLY;BYDAY=FR;BYSETPOS=-1;COUNT=3",
            "2026-10-30 15:00",
            "2027-06-01 00:00"
        ),
        ["2026-10-30 Fri", "2026-11-27 Fri", "2026-12-25 Fri"]
    );
    // Her ayın 2. Salısı (Outlook'un diğer yazımı).
    assert_eq!(
        occ(
            "FREQ=MONTHLY;BYDAY=2TU;COUNT=2",
            "2026-10-13 09:00",
            "2027-06-01 00:00"
        ),
        ["2026-10-13 Tue", "2026-11-10 Tue"]
    );
    // Ayın 31'i olmayan aylar atlanır.
    assert_eq!(
        occ(
            "FREQ=MONTHLY;BYMONTHDAY=31",
            "2026-10-31 09:00",
            "2027-01-02 00:00"
        ),
        ["2026-10-31 Sat", "2026-12-31 Thu"]
    );
    // Hafta içi her gün; pencere sonunda durur (sonsuz kural).
    assert_eq!(
        occ(
            "FREQ=DAILY;BYDAY=MO,TU,WE,TH,FR",
            "2026-10-08 09:00",
            "2026-10-13 23:00"
        ),
        [
            "2026-10-08 Thu",
            "2026-10-09 Fri",
            "2026-10-12 Mon",
            "2026-10-13 Tue"
        ]
    );
    // Yıllık: Ekim'in ilk Pazartesisi.
    assert_eq!(
        occ(
            "FREQ=YEARLY;BYMONTH=10;BYDAY=1MO;COUNT=2",
            "2026-10-05 09:00",
            "2030-01-01 00:00"
        ),
        ["2026-10-05 Mon", "2027-10-04 Mon"]
    );
}

const OUTLOOK: &str = "BEGIN:VCALENDAR\r
METHOD:PUBLISH\r
PRODID:Microsoft Exchange Server 2010\r
VERSION:2.0\r
BEGIN:VTIMEZONE\r
TZID:Turkey Standard Time\r
BEGIN:STANDARD\r
DTSTART:16010101T000000\r
TZOFFSETFROM:+0300\r
TZOFFSETTO:+0300\r
END:STANDARD\r
BEGIN:DAYLIGHT\r
DTSTART:16010101T000000\r
TZOFFSETFROM:+0300\r
TZOFFSETTO:+0300\r
END:DAYLIGHT\r
END:VTIMEZONE\r
BEGIN:VTIMEZONE\r
TZID:W. Europe Standard Time\r
BEGIN:STANDARD\r
DTSTART:16010101T030000\r
TZOFFSETFROM:+0200\r
TZOFFSETTO:+0100\r
RRULE:FREQ=YEARLY;INTERVAL=1;BYDAY=-1SU;BYMONTH=10\r
END:STANDARD\r
BEGIN:DAYLIGHT\r
DTSTART:16010101T020000\r
TZOFFSETFROM:+0100\r
TZOFFSETTO:+0200\r
RRULE:FREQ=YEARLY;INTERVAL=1;BYDAY=-1SU;BYMONTH=3\r
END:DAYLIGHT\r
END:VTIMEZONE\r
BEGIN:VEVENT\r
RRULE:FREQ=WEEKLY;UNTIL=20261102T070000Z;INTERVAL=1;BYDAY=MO;WKST=MO\r
EXDATE;TZID=Turkey Standard Time:20261019T100000\r
UID:040000008200E00074C5B7101A82E0080000000001\r
SUMMARY;LANGUAGE=tr-TR:Togg haftalık\\, durum\r
DTSTART;TZID=Turkey Standard Time:20261005T100000\r
DTEND;TZID=Turkey Standard Time:20261005T103000\r
LOCATION;LANGUAGE=tr-TR:Microsoft Teams Toplantısı\r
X-MICROSOFT-CDO-BUSYSTATUS:BUSY\r
END:VEVENT\r
BEGIN:VEVENT\r
RECURRENCE-ID;TZID=Turkey Standard Time:20261012T100000\r
UID:040000008200E00074C5B7101A82E0080000000001\r
SUMMARY:Togg haftalık\\, durum\r
DTSTART;TZID=Turkey Standard Time:20261012T140000\r
DTEND;TZID=Turkey Standard Time:20261012T150000\r
LOCATION:Microsoft Teams Toplantısı\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:berlin\r
SUMMARY:Berlin atölye\r
DTSTART;TZID=W. Europe Standard Time:20261026T090000\r
DTEND;TZID=W. Europe Standard Time:20261026T110000\r
LOCATION:Toplantı odası 3\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:summer\r
SUMMARY:Yaz saati\r
DTSTART;TZID=W. Europe Standard Time:20260715T090000\r
DURATION:PT45M\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:allday\r
SUMMARY:Bayram\r
DTSTART;VALUE=DATE:20261029\r
DTEND;VALUE=DATE:20261030\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:free\r
SUMMARY:Öğle\r
DTSTART:20261005T090000Z\r
DTEND:20261005T100000Z\r
X-MICROSOFT-CDO-BUSYSTATUS:FREE\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:cancel\r
SUMMARY:İptal edildi: Kickoff\r
DTSTART:20261006T090000Z\r
DTEND:20261006T100000Z\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:long-desc\r
SUMMARY:Uzun açıklama\r
DTSTART:20261007T090000Z\r
DTEND:20261007T093000Z\r
DESCRIPTION:Katıl: https://teams.micro\r
 soft.com/l/meetup-join/abc\r
END:VEVENT\r
END:VCALENDAR\r
";

#[test]
fn parses_outlook_calendar_with_zones_and_exceptions() {
    let cal = Calendar::parse(OUTLOOK);
    let got: Vec<(String, String, String, bool)> = cal
        .meetings(utc("2026-01-01 00:00"), utc("2027-01-01 00:00"))
        .into_iter()
        .map(|m| {
            (
                m.start.format("%m-%d %H:%M").to_string(),
                m.end.format("%H:%M").to_string(),
                m.subject,
                m.online,
            )
        })
        .collect();
    let row = |s: &str, e: &str, subject: &str, online: bool| {
        (s.to_string(), e.to_string(), subject.to_string(), online)
    };
    assert_eq!(
        got,
        [
            // Yaz saatinde Berlin UTC+2.
            row("07-15 07:00", "07:45", "Yaz saati", false),
            // Türkiye UTC+3: 10:00 → 07:00Z.
            row("10-05 07:00", "07:30", "Togg haftalık, durum", true),
            // Satır katlanması birleştirilir: açıklamadaki Teams linki.
            row("10-07 09:00", "09:30", "Uzun açıklama", true),
            // 12 Ekim tek seferlik 14:00'e alındı.
            row("10-12 11:00", "12:00", "Togg haftalık, durum", true),
            // 19 Ekim atlandı (EXDATE). 26 Ekim'de Berlin kış saatine döndü (UTC+1).
            row("10-26 07:00", "07:30", "Togg haftalık, durum", true),
            row("10-26 08:00", "10:00", "Berlin atölye", false),
            // UNTIL 2 Kasım 07:00Z (dahil).
            row("11-02 07:00", "07:30", "Togg haftalık, durum", true),
        ]
    );
    // Aralık süzgeci: yalnızca kesişenler.
    let day = cal.meetings(utc("2026-10-26 00:00"), utc("2026-10-26 07:15"));
    assert_eq!(day.len(), 1);
    assert_eq!(day[0].uid, "040000008200E00074C5B7101A82E0080000000001");
}

#[test]
fn parses_small_pieces() {
    assert_eq!(parse_offset("+0300"), Some(10_800));
    assert_eq!(parse_offset("-0430"), Some(-16_200));
    assert_eq!(parse_duration("PT1H30M"), Some(Duration::minutes(90)));
    assert_eq!(parse_duration("P1DT2H"), Some(Duration::hours(26)));
    assert_eq!(parse_byday("-1SU"), Some((-1, Weekday::Sun)));
    assert_eq!(parse_byday("MO"), Some((0, Weekday::Mon)));
    assert_eq!(unescape(r"a\, b\; c\nd\\e"), "a, b; c\nd\\e");
    let line = parse_line(r#"DTSTART;TZID="Ev: Saat":20261005T100000"#).unwrap();
    assert_eq!(line.param("TZID"), Some("Ev: Saat"));
    assert_eq!(line.value, "20261005T100000");
}

#[test]
fn parses_organizer_and_attendees() {
    // "Tüm ayrıntılar" düzeyinde yayımlanan Outlook takvimi: CN parametreleri (tırnaklı,
    // içinde ':' ve ';'), büyük harfli MAILTO, katlanmış satır, adresi olmayan oda.
    let text = "BEGIN:VCALENDAR\r
BEGIN:VEVENT\r
UID:acme-weekly\r
SUMMARY:Loyalty haftalık\r
DTSTART:20261005T090000Z\r
DTEND:20261005T100000Z\r
ORGANIZER;CN=\"Yılmaz, Ayşe: PM\":mailto:Ayse.Yilmaz@Acme.com\r
ATTENDEE;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE;CN=Ali Veli:MAILTO:ali@\r
 acme.com\r
ATTENDEE;CN=Ben;RSVP=TRUE:mailto:me@kum.dev\r
ATTENDEE;CUTYPE=RESOURCE;CN=Toplantı Odası 3:Toplantı Odası 3\r
ATTENDEE;CN=E-posta param;EMAIL=veli@partner.com.tr:urn:uuid:1234\r
ATTENDEE;CN=Tekrar:mailto:ali@acme.com\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:no-details\r
SUMMARY:Meşgul\r
DTSTART:20261005T110000Z\r
DTEND:20261005T113000Z\r
END:VEVENT\r
END:VCALENDAR\r
";
    let cal = Calendar::parse(text);
    let got = cal.meetings(utc("2026-10-05 00:00"), utc("2026-10-06 00:00"));
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].organizer.as_deref(), Some("ayse.yilmaz@acme.com"));
    assert_eq!(
        got[0].attendees,
        ["ali@acme.com", "me@kum.dev", "veli@partner.com.tr"]
    );
    // Katılımcı bilgisi yayımlanmamışsa boş kalır.
    assert_eq!(got[1].organizer, None);
    assert!(got[1].attendees.is_empty());
    // Seri başına bir örnek de aynı bilgileri taşır.
    let series = cal.series();
    assert_eq!(series.len(), 2);
    assert_eq!(series[0].attendees.len(), 3);
}

#[test]
fn absurd_durations_and_intervals_do_not_panic() {
    assert_eq!(parse_duration("P200000000D"), None);
    assert_eq!(parse_duration("P99999999999999999W"), None);
    assert_eq!(parse_duration("PT9223372036854775807S"), None);
    assert_eq!(parse_duration("P40D"), None);
    assert_eq!(parse_duration("-P1W"), Some(-Duration::weeks(1)));
    let text = "BEGIN:VCALENDAR\r
BEGIN:VEVENT\r
UID:dur\r
SUMMARY:Bozuk süre\r
DTSTART:20261005T090000Z\r
DURATION:P200000000D\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:far\r
SUMMARY:Uzak bitiş\r
DTSTART:00010101T090000Z\r
DTEND:99991231T090000Z\r
RRULE:FREQ=YEARLY;INTERVAL=4294967295\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:interval\r
SUMMARY:Dev aralık\r
DTSTART:20261005T090000Z\r
DTEND:20261005T100000Z\r
RRULE:FREQ=DAILY;INTERVAL=4294967295;BYDAY=99999MO\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:monthly\r
SUMMARY:Aylık\r
DTSTART:20261005T090000Z\r
DTEND:20261005T100000Z\r
RRULE:FREQ=MONTHLY;INTERVAL=4294967295;BYDAY=300MO\r
END:VEVENT\r
END:VCALENDAR\r
";
    let cal = Calendar::parse(text);
    let got = cal.meetings(utc("2026-01-01 00:00"), utc("2027-01-01 00:00"));
    // Süresi okunamayan etkinlik sıfır uzunluktadır, toplantı sayılmaz; olmayan
    // "300. Pazartesi" hiç açılmaz.
    assert_eq!(
        got.iter().map(|m| m.uid.as_str()).collect::<Vec<_>>(),
        ["interval"]
    );
    cal.series();
}

#[test]
fn alarm_lines_do_not_leak_into_the_event() {
    let text = "BEGIN:VCALENDAR\r
BEGIN:VEVENT\r
UID:alarm\r
SUMMARY:Sprint planlama\r
DTSTART:20261005T090000Z\r
BEGIN:VALARM\r
ACTION:EMAIL\r
SUMMARY:Alarm notification\r
ATTENDEE:mailto:alarm@acme.com\r
DURATION:PT5M\r
TRIGGER:-PT15M\r
END:VALARM\r
DURATION:PT30M\r
END:VEVENT\r
END:VCALENDAR\r
";
    let cal = Calendar::parse(text);
    let got = cal.meetings(utc("2026-10-05 00:00"), utc("2026-10-06 00:00"));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].subject, "Sprint planlama");
    assert!(got[0].attendees.is_empty());
    assert_eq!(got[0].end - got[0].start, Duration::minutes(30));
}

#[test]
fn ended_daylight_rules_do_not_apply_after_until() {
    // tzurl / Google biçimi: Türkiye 2016'da yaz saatinden çıktı, kalıcı +03 oldu.
    let text = "BEGIN:VCALENDAR\r
BEGIN:VTIMEZONE\r
TZID:Europe/Istanbul\r
BEGIN:DAYLIGHT\r
TZOFFSETFROM:+0200\r
TZOFFSETTO:+0300\r
DTSTART:20110328T030000\r
RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU;UNTIL=20160327T010000Z\r
END:DAYLIGHT\r
BEGIN:STANDARD\r
TZOFFSETFROM:+0300\r
TZOFFSETTO:+0200\r
DTSTART:20111030T040000\r
RRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU;UNTIL=20151108T010000Z\r
END:STANDARD\r
BEGIN:STANDARD\r
TZOFFSETFROM:+0300\r
TZOFFSETTO:+0300\r
DTSTART:20160907T000000\r
END:STANDARD\r
END:VTIMEZONE\r
BEGIN:VEVENT\r
UID:winter\r
SUMMARY:Kış toplantısı\r
DTSTART;TZID=Europe/Istanbul:20261210T100000\r
DTEND;TZID=Europe/Istanbul:20261210T110000\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:old\r
SUMMARY:Eski kış\r
DTSTART;TZID=Europe/Istanbul:20141210T100000\r
DTEND;TZID=Europe/Istanbul:20141210T110000\r
END:VEVENT\r
END:VCALENDAR\r
";
    let cal = Calendar::parse(text);
    let got = cal.meetings(utc("2014-01-01 00:00"), utc("2027-01-01 00:00"));
    let starts: Vec<_> = got.iter().map(|m| m.start).collect();
    // 2014 kışı +02, 2026 kışı +03.
    assert_eq!(starts, [utc("2014-12-10 08:00"), utc("2026-12-10 07:00")]);
}

#[test]
fn malformed_non_ascii_values_do_not_panic() {
    assert_eq!(parse_naive("20240101Tİ00000"), None);
    assert_eq!(parse_offset("+0İ00"), None);
    assert_eq!(parse_byday("Öx"), None);
}
