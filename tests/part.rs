use bytes::Bytes;
use ump_stream::{decode_all, Decoder, Frame, Next, Part, PartError, PartKind, PartReader};

fn sample() -> Vec<Part> {
    vec![
        Part::new(20, b"init".as_slice()),
        Part::new(21, vec![0xab; 200]),
        Part::new(21, vec![0xcd; 100_000]),
        Part::new(2_097_152, vec![1; 2_097_152]),
        Part::new(u32::MAX, Bytes::new()),
        Part::new(22, Bytes::new()),
    ]
}

fn concat(parts: &[Part]) -> Vec<u8> {
    let mut out = Vec::new();
    for p in parts {
        p.encode_into(&mut out).unwrap();
    }
    out
}

#[test]
fn header_varint_known_bytes() {
    assert_eq!(&Part::new(20, b"abc".as_slice()).encode().unwrap()[..2], &[0x14, 0x03]);
    assert_eq!(&Part::new(21, vec![0; 200]).encode().unwrap()[..3], &[0x15, 0x88, 0x03]);
    assert_eq!(&Part::new(21, vec![0; 100_000]).encode().unwrap()[..4], &[0x15, 0xc0, 0x35, 0x0c]);
    assert_eq!(&Part::new(2_097_152, Bytes::new()).encode().unwrap(), &[0xe0, 0x00, 0x00, 0x02, 0x00]);
    assert_eq!(&Part::new(u32::MAX, Bytes::new()).encode().unwrap(), &[0xf0, 0xff, 0xff, 0xff, 0xff, 0x00]);
}

#[test]
fn sync_roundtrip() {
    let parts = sample();
    let bytes = concat(&parts);
    assert_eq!(decode_all(&bytes).unwrap(), parts);
    assert!(matches!(decode_all(&bytes[..bytes.len() - 1]), Err(PartError::Truncated)));
    assert!(matches!(decode_all(&bytes[..1]), Err(PartError::Truncated)));
    assert!(decode_all(&[]).unwrap().is_empty());
    let (first, n) = Part::decode(&bytes).unwrap();
    assert_eq!((first, n), (parts[0].clone(), 6));
}

#[test]
fn decoder_push_in_odd_chunks() {
    let parts = sample();
    let bytes = concat(&parts);
    let mut dec = Decoder::new();
    let mut got = Vec::new();
    for chunk in bytes.chunks(7) {
        dec.push(chunk);
        while let Frame::Part(p) = dec.take() {
            got.push(p);
        }
    }
    dec.close().unwrap();
    assert_eq!(got, parts);

    let mut dec = Decoder::new();
    dec.push(&bytes[..3]);
    assert_eq!(dec.take(), Frame::Incomplete);
    assert!(matches!(dec.close(), Err(PartError::Truncated)));
}

#[tokio::test]
async fn reader_across_single_byte_reads() {
    let parts = sample();
    let bytes = concat(&parts);
    let mut r = PartReader::new(tokio::io::BufReader::with_capacity(1, bytes.as_slice()));
    let mut got = Vec::new();
    while let Next::Part(p) = r.next().await.unwrap() {
        got.push(p);
    }
    assert_eq!(got, parts);
    assert_eq!(r.into_inner().buffer().len(), 0);
}

#[tokio::test]
async fn reader_reports_truncation() {
    let bytes = concat(&sample());
    let mut r = PartReader::new(&bytes[..10]);
    assert!(matches!(r.next().await.unwrap(), Next::Part(Part { kind: PartKind(20), .. })));
    assert!(matches!(r.next().await, Err(PartError::Truncated)));
}
