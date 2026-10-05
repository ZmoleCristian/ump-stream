//! Real captures: `tests/fixtures/real/req_*.bin` (protobuf request bodies) and
//! `resp_*.bin` (UMP part streams). `--nocapture` dumps what was found.

mod common;

use std::collections::BTreeMap;

use ump_stream::{decode_all, Message, PartKind, Value};

#[test]
fn every_request_decodes_with_required_fields() {
    for (name, data) in common::real("req_") {
        let m = Message::decode(&data).unwrap_or_else(|e| panic!("{name}: {e}"));
        let numbers: Vec<u32> = m.fields.iter().map(|f| f.number).collect();
        println!("{name}: {} bytes, fields {numbers:?}", data.len());
        for required in [1, 5, 19] {
            m.get(required).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }
}

#[test]
fn every_response_decodes_completely() {
    let files = common::real("resp_");
    if files.is_empty() {
        return;
    }
    let mut seen: BTreeMap<PartKind, (usize, usize)> = BTreeMap::new();
    for (name, data) in files {
        let parts = decode_all(&data).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!parts.is_empty(), "{name}: empty");
        let kinds: Vec<u32> = parts.iter().map(|p| p.kind.0).collect();
        println!("{name}: {} bytes, kinds {kinds:?}", data.len());
        for p in &parts {
            let e = seen.entry(p.kind).or_insert((0, 0));
            e.0 += 1;
            e.1 += p.payload.len();
            if let Ok(m) = p.message() {
                if p.payload.len() <= 4096 && !m.fields.is_empty() {
                    print!("{}", dump(&m, 2));
                }
            }
        }
    }
    for (kind, (count, bytes)) in &seen {
        println!("kind {kind}: {count} parts, {bytes} bytes");
    }
    let observed: Vec<PartKind> = seen.keys().copied().collect();
    assert_eq!(observed, PartKind::OBSERVED, "update PartKind constants in src/part.rs");
}

/// Captured requests pad length varints (`c0 81 00` for 192), so a canonical
/// encoder cannot reproduce them byte for byte. What must hold: decoding is
/// lossless and re-encoding is canonical (never larger, re-decodes equal).
#[test]
fn request_roundtrip_is_lossless_and_canonical() {
    for (name, data) in common::real("req_") {
        let m = Message::decode(&data).unwrap();
        let enc = m.encode();
        assert_eq!(Message::decode(&enc).unwrap(), m, "{name}");
        assert!(enc.len() <= data.len(), "{name}: {} > {}", enc.len(), data.len());
        assert_eq!(Message::decode(&enc).unwrap().encode(), enc, "{name}: canonical form must be a fixpoint");
        println!("{name}: {} -> {} bytes ({} bytes of varint padding)", data.len(), enc.len(), data.len() - enc.len());
    }
}

#[test]
fn redirect_part_carries_url_in_field_1() {
    let Some((_, data)) = common::real("resp_001").pop() else { return };
    let parts = decode_all(&data).unwrap();
    let redirect = parts.iter().find(|p| p.kind == PartKind::REDIRECT).expect("resp_001 has no redirect");
    let url = redirect.message().unwrap().get(1).unwrap().as_str().unwrap().to_owned();
    assert!(url.starts_with("https://"), "{url}");
}

fn dump(m: &Message, depth: usize) -> String {
    let pad = " ".repeat(depth * 2);
    let mut out = String::new();
    for f in &m.fields {
        out.push_str(&format!("{pad}{}: ", f.number));
        match &f.value {
            Value::Varint(v) => out.push_str(&format!("{v}\n")),
            Value::Fixed64(v) => out.push_str(&format!("0x{v:016x}\n")),
            Value::Fixed32(v) => out.push_str(&format!("0x{v:08x}\n")),
            Value::Bytes(b) => match nested(b) {
                Some(inner) => out.push_str(&format!("{{\n{}{pad}}}\n", dump(&inner, depth + 1))),
                None => match std::str::from_utf8(b) {
                    Ok(s) if !s.chars().any(char::is_control) => out.push_str(&format!("{s:?}\n")),
                    _ => out.push_str(&format!("<{} bytes>\n", b.len())),
                },
            },
        }
    }
    out
}

fn nested(b: &[u8]) -> Option<Message> {
    if b.is_empty() {
        return None;
    }
    let m = Message::decode(b).ok()?;
    m.fields.iter().all(|f| (1..1 << 16).contains(&f.number)).then_some(m)
}
