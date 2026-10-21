//! Review probes for the INI writer (Phase 4 slice 3).
use agora_core::game_ini::IniDocument;

const AWKWARD: &[u8] = b"\xEF\xBB\xBF; header comment\r\n\
[General]\r\n\
sLanguage=ENGLISH\r\n\
bare line without equals\r\n\
sPath = a=b=c ; not a comment\r\n\
[\r\n\
[ Display ]\r\n\
iSize W=1920\r\n\
\r\n\
[general]\r\n\
fDup=1\r\n\
=no key\r\n";

#[test]
fn probe_round_trip_without_changes_is_byte_identical() {
    assert_eq!(IniDocument::parse(AWKWARD).render(), AWKWARD);
    assert_eq!(IniDocument::parse(b"").render(), b"");
    let no_newline = b"[A]\nk=v";
    assert_eq!(IniDocument::parse(no_newline).render(), no_newline);
}

#[test]
fn probe_set_changes_one_line_only() {
    let mut doc = IniDocument::parse(AWKWARD);
    doc.set("Display", "iSize W", "2560").unwrap();
    let out = doc.render();
    let before: Vec<&[u8]> = AWKWARD.split(|b| *b == b'\n').collect();
    let after: Vec<&[u8]> = out.split(|b| *b == b'\n').collect();
    assert_eq!(before.len(), after.len(), "no line added or removed");
    let changed: Vec<usize> = (0..before.len())
        .filter(|i| before[*i] != after[*i])
        .collect();
    assert_eq!(changed.len(), 1, "exactly one line changed: {changed:?}");
    assert!(String::from_utf8_lossy(after[changed[0]]).contains("2560"));
    assert!(out.starts_with(b"\xEF\xBB\xBF"), "BOM kept");
}

#[test]
fn probe_values_keep_equals_signs_and_semicolons() {
    let doc = IniDocument::parse(AWKWARD);
    let v = doc.get("general", "spath").unwrap();
    assert!(v.contains("a=b=c"), "value was {v:?}");
}

#[test]
fn probe_empty_section_or_key_is_refused() {
    let mut doc = IniDocument::parse(AWKWARD);
    let original = doc.render();
    assert!(doc.set("", "k", "v").is_err() || doc.get("", "k").is_some());
    assert!(
        doc.set("General", "", "v").is_err(),
        "an empty key must be refused"
    );
    assert!(
        doc.set("General", "a\nb", "v").is_err(),
        "a newline in a key must be refused"
    );
    assert!(
        doc.set("General", "k", "v\r\n[Injected]").is_err(),
        "a newline in a value must be refused"
    );
    let _ = original;
}
