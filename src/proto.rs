use std::num::TryFromIntError;
use std::str::Utf8Error;

#[derive(Debug, thiserror::Error)]
pub enum ProtoError {
    #[error("message ended mid-field")]
    Truncated,
    #[error("varint longer than 10 bytes")]
    Overflow,
    #[error("unsupported wire type {0}")]
    WireType(u64),
    #[error("value does not fit: {0}")]
    TooWide(#[from] TryFromIntError),
    #[error("field {0} missing")]
    Missing(u32),
    #[error("expected wire type {expected}, found {found}")]
    Mismatch { expected: u64, found: u64 },
    #[error("not utf-8: {0}")]
    Utf8(#[from] Utf8Error),
}

fn len_u64(n: usize) -> u64 {
    match u64::try_from(n) {
        Ok(v) => v,
        Err(e) => panic!("usize does not fit u64 on this target: {e}"),
    }
}

fn fixed<const N: usize>(buf: &[u8]) -> Result<[u8; N], ProtoError> {
    let Some(&b) = buf.first_chunk::<N>() else { return Err(ProtoError::Truncated) };
    Ok(b)
}

pub fn put_varint(mut v: u64, out: &mut Vec<u8>) {
    while v >= 0x80 {
        out.push(v.to_le_bytes()[0] | 0x80);
        v >>= 7;
    }
    out.push(v.to_le_bytes()[0]);
}

/// Decode a protobuf varint from the front of `buf`, returning it with the bytes consumed.
pub fn get_varint(buf: &[u8]) -> Result<(u64, usize), ProtoError> {
    let mut v = 0u64;
    for (i, &b) in buf.iter().enumerate().take(10) {
        v |= u64::from(b & 0x7f) << (7 * i);
        if b < 0x80 {
            return Ok((v, i + 1));
        }
    }
    if buf.len() < 10 {
        Err(ProtoError::Truncated)
    } else {
        Err(ProtoError::Overflow)
    }
}

/// A wire-level value. Nested messages are `Bytes` until [`Value::as_message`] is asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Varint(u64),
    Fixed64(u64),
    Bytes(Vec<u8>),
    Fixed32(u32),
}

impl Value {
    pub fn wire_type(&self) -> u64 {
        match self {
            Value::Varint(_) => 0,
            Value::Fixed64(_) => 1,
            Value::Bytes(_) => 2,
            Value::Fixed32(_) => 5,
        }
    }

    pub fn as_u64(&self) -> Result<u64, ProtoError> {
        match self {
            Value::Varint(v) => Ok(*v),
            other => Err(ProtoError::Mismatch { expected: 0, found: other.wire_type() }),
        }
    }

    pub fn as_bytes(&self) -> Result<&[u8], ProtoError> {
        match self {
            Value::Bytes(b) => Ok(b),
            other => Err(ProtoError::Mismatch { expected: 2, found: other.wire_type() }),
        }
    }

    pub fn as_str(&self) -> Result<&str, ProtoError> {
        Ok(std::str::from_utf8(self.as_bytes()?)?)
    }

    pub fn as_message(&self) -> Result<Message, ProtoError> {
        Message::decode(self.as_bytes()?)
    }

    pub fn encode_into(&self, number: u32, out: &mut Vec<u8>) {
        put_varint(u64::from(number) << 3 | self.wire_type(), out);
        match self {
            Value::Varint(v) => put_varint(*v, out),
            Value::Fixed64(v) => out.extend(v.to_le_bytes()),
            Value::Fixed32(v) => out.extend(v.to_le_bytes()),
            Value::Bytes(b) => {
                put_varint(len_u64(b.len()), out);
                out.extend_from_slice(b);
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub number: u32,
    pub value: Value,
}

/// Schema-less message: ordered `(field number, value)` pairs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub fields: Vec<Field>,
}

impl Message {
    pub fn decode(buf: &[u8]) -> Result<Self, ProtoError> {
        let mut fields = Vec::new();
        let mut rest = buf;
        while !rest.is_empty() {
            let (tag, n) = get_varint(rest)?;
            rest = &rest[n..];
            let number = u32::try_from(tag >> 3)?;
            let (value, used) = match tag & 7 {
                0 => {
                    let (v, n) = get_varint(rest)?;
                    (Value::Varint(v), n)
                }
                1 => (Value::Fixed64(u64::from_le_bytes(fixed::<8>(rest)?)), 8),
                2 => {
                    let (len, n) = get_varint(rest)?;
                    let end = n.checked_add(usize::try_from(len)?).ok_or(ProtoError::Truncated)?;
                    let b = rest.get(n..end).ok_or(ProtoError::Truncated)?;
                    (Value::Bytes(b.to_vec()), end)
                }
                5 => (Value::Fixed32(u32::from_le_bytes(fixed::<4>(rest)?)), 4),
                wire => return Err(ProtoError::WireType(wire)),
            };
            rest = &rest[used..];
            fields.push(Field { number, value });
        }
        Ok(Message { fields })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode_into(&mut out);
        out
    }

    pub fn encode_into(&self, out: &mut Vec<u8>) {
        for f in &self.fields {
            f.value.encode_into(f.number, out);
        }
    }

    pub fn push(&mut self, number: u32, value: Value) {
        self.fields.push(Field { number, value });
    }

    /// First occurrence of `number`.
    pub fn get(&self, number: u32) -> Result<&Value, ProtoError> {
        self.get_all(number).next().ok_or(ProtoError::Missing(number))
    }

    pub fn get_all(&self, number: u32) -> impl Iterator<Item = &Value> {
        self.fields.iter().filter(move |f| f.number == number).map(|f| &f.value)
    }
}

/// Builds a request body field by field. Numbers and types are whatever the server expects.
#[derive(Clone, Debug)]
pub struct RequestBuilder {
    buf: Vec<u8>,
}

impl RequestBuilder {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    fn tag(mut self, number: u32, wire_type: u64) -> Self {
        put_varint(u64::from(number) << 3 | wire_type, &mut self.buf);
        self
    }

    /// uint32 / uint64 / enum.
    pub fn varint(self, number: u32, v: u64) -> Self {
        let mut s = self.tag(number, 0);
        put_varint(v, &mut s.buf);
        s
    }

    /// int32 / int64: two's complement, ten bytes when negative.
    pub fn int(self, number: u32, v: i64) -> Self {
        self.varint(number, u64::from_ne_bytes(v.to_ne_bytes()))
    }

    /// sint32 / sint64: zigzag.
    pub fn sint(self, number: u32, v: i64) -> Self {
        self.varint(number, u64::from_ne_bytes(((v << 1) ^ (v >> 63)).to_ne_bytes()))
    }

    pub fn bool(self, number: u32, v: bool) -> Self {
        self.varint(number, u64::from(v))
    }

    pub fn fixed64(self, number: u32, v: u64) -> Self {
        let mut s = self.tag(number, 1);
        s.buf.extend(v.to_le_bytes());
        s
    }

    pub fn fixed32(self, number: u32, v: u32) -> Self {
        let mut s = self.tag(number, 5);
        s.buf.extend(v.to_le_bytes());
        s
    }

    pub fn double(self, number: u32, v: f64) -> Self {
        self.fixed64(number, v.to_bits())
    }

    pub fn float(self, number: u32, v: f32) -> Self {
        self.fixed32(number, v.to_bits())
    }

    pub fn bytes(self, number: u32, v: impl AsRef<[u8]>) -> Self {
        let v = v.as_ref();
        let mut s = self.tag(number, 2);
        put_varint(len_u64(v.len()), &mut s.buf);
        s.buf.extend_from_slice(v);
        s
    }

    pub fn string(self, number: u32, v: impl AsRef<str>) -> Self {
        self.bytes(number, v.as_ref().as_bytes())
    }

    /// Nested message built by `f`.
    pub fn message(self, number: u32, f: impl FnOnce(Self) -> Self) -> Self {
        let inner = f(Self::new()).build();
        self.bytes(number, inner)
    }

    /// Packed repeated varints.
    pub fn packed(self, number: u32, vs: impl IntoIterator<Item = u64>) -> Self {
        let mut tmp = Vec::new();
        for v in vs {
            put_varint(v, &mut tmp);
        }
        self.bytes(number, tmp)
    }

    /// A wire value as-is, e.g. copied out of a decoded server message.
    pub fn field(mut self, number: u32, v: &Value) -> Self {
        v.encode_into(number, &mut self.buf);
        self
    }

    /// Append every field of a decoded message.
    pub fn merge(mut self, m: &Message) -> Self {
        m.encode_into(&mut self.buf);
        self
    }

    pub fn build(self) -> Vec<u8> {
        self.buf
    }
}
