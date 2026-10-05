use ump_stream::proto::{get_varint, put_varint};
use ump_stream::{Message, ProtoError, RequestBuilder, Value};

#[test]
fn varint_roundtrip() {
    for v in [0, 1, 127, 128, 300, u64::from(u32::MAX), u64::MAX] {
        let mut b = Vec::new();
        put_varint(v, &mut b);
        assert_eq!(get_varint(&b).unwrap(), (v, b.len()));
    }
    assert!(matches!(get_varint(&[0x80]), Err(ProtoError::Truncated)));
    assert!(matches!(get_varint(&[0xff; 11]), Err(ProtoError::Overflow)));
}

#[test]
fn message_roundtrip() {
    let mut m = Message { fields: Vec::new() };
    m.push(1, Value::Varint(150));
    m.push(2, Value::Bytes(b"hi".to_vec()));
    m.push(3, Value::Fixed32(7));
    m.push(4, Value::Fixed64(u64::MAX));
    let enc = m.encode();
    assert_eq!(&enc[..3], &[0x08, 0x96, 0x01]);
    assert_eq!(Message::decode(&enc).unwrap(), m);
    assert_eq!(m.get(2).unwrap().as_str().unwrap(), "hi");
    assert!(matches!(m.get(9), Err(ProtoError::Missing(9))));
    assert!(matches!(m.get(2).unwrap().as_u64(), Err(ProtoError::Mismatch { expected: 0, found: 2 })));
    assert!(matches!(m.get(1).unwrap().as_bytes(), Err(ProtoError::Mismatch { expected: 2, found: 0 })));
}

#[test]
fn decode_rejects_bad_input() {
    assert!(matches!(Message::decode(&[0x0b]), Err(ProtoError::WireType(3))));
    assert!(matches!(Message::decode(&[0x12, 0x05, 0x01]), Err(ProtoError::Truncated)));
    assert!(matches!(Message::decode(&[0x09, 0x01]), Err(ProtoError::Truncated)));
    assert!(matches!(Value::Bytes(vec![0xff]).as_str(), Err(ProtoError::Utf8(_))));
}

#[test]
fn builder_matches_wire_format() {
    let b = RequestBuilder::new()
        .varint(1, 150)
        .string(2, "testing")
        .message(3, |m| m.sint(1, -1).bool(2, true))
        .packed(4, [3, 270, 86942])
        .fixed32(5, 1)
        .fixed64(6, 2)
        .build();
    let expect = [
        0x08, 0x96, 0x01, //
        0x12, 0x07, b't', b'e', b's', b't', b'i', b'n', b'g', //
        0x1a, 0x04, 0x08, 0x01, 0x10, 0x01, //
        0x22, 0x06, 0x03, 0x8e, 0x02, 0x9e, 0xa7, 0x05, //
        0x2d, 1, 0, 0, 0, //
        0x31, 2, 0, 0, 0, 0, 0, 0, 0,
    ];
    assert_eq!(b, expect);
    let m = Message::decode(&b).unwrap();
    assert_eq!(m.get(3).unwrap().as_message().unwrap().get(2).unwrap().as_u64().unwrap(), 1);
    assert_eq!(RequestBuilder::new().merge(&m).build(), b);
    assert_eq!(RequestBuilder::new().field(1, m.get(1).unwrap()).build(), &b[..3]);
}

#[test]
fn negative_int_is_ten_bytes() {
    let b = RequestBuilder::new().int(1, -1).build();
    assert_eq!(b.len(), 11);
    assert_eq!(Message::decode(&b).unwrap().get(1).unwrap().as_u64().unwrap(), u64::MAX);
}

#[test]
fn floats_are_fixed() {
    let b = RequestBuilder::new().float(1, 1.5).double(2, -2.0).build();
    let m = Message::decode(&b).unwrap();
    assert_eq!(m.get(1).unwrap(), &Value::Fixed32(1.5f32.to_bits()));
    assert_eq!(m.get(2).unwrap(), &Value::Fixed64((-2.0f64).to_bits()));
}
