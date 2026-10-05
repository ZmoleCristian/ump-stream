use std::fmt;
use std::num::TryFromIntError;

use bytes::{Buf, Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::proto::{Message, ProtoError};

#[derive(Debug, thiserror::Error)]
pub enum PartError {
    #[error("stream ended inside a part")]
    Truncated,
    #[error("payload length exceeds u32: {0}")]
    TooLarge(#[from] TryFromIntError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Raw part type number. Its meaning belongs to the server you talk to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PartKind(pub u32);

/// Kinds observed in the captured streams under `tests/fixtures/real`. Names
/// follow the payload shape; where the semantics are not confirmed by the
/// payload itself the doc says so.
impl PartKind {
    /// Protobuf. Precedes each run of [`Self::MEDIA`]: 1 header id, 2 video id,
    /// 3 itag, 4 format lmt, 6 start byte, 8 is-init flag, 10 byte length,
    /// 11 start ms, 12 duration ms, 13 `{itag, lmt}`, 14 sequence, 15 time range.
    pub const MEDIA_HEADER: PartKind = PartKind(20);
    /// Raw media bytes belonging to the preceding [`Self::MEDIA_HEADER`].
    pub const MEDIA: PartKind = PartKind(21);
    /// Empty. Closes the current [`Self::MEDIA_HEADER`] run.
    pub const MEDIA_END: PartKind = PartKind(22);
    /// Protobuf, once per response: 1/2 audio and video readahead ms,
    /// 3 and 4 timeouts ms, 7 playback cookie carrying the selected formats
    /// (7 video `{itag, lmt}`, 8 audio `{itag, lmt}`, 20 opaque 32 bytes).
    pub const NEXT_REQUEST_POLICY: PartKind = PartKind(35);
    /// Protobuf, once per selected format: 1 video id, 2 `{itag, lmt}`,
    /// 3 end ms, 4 segment count, 5 mime type, 6 init byte range,
    /// 7 index byte range, 9 duration, 10 duration timescale.
    pub const FORMAT_INITIALIZATION_METADATA: PartKind = PartKind(42);
    /// Protobuf whose field 1 is the next endpoint URL.
    pub const REDIRECT: PartKind = PartKind(43);
    /// Protobuf, once per response: two ms windows `{1: {1: 0, 2: 1200}, 2: {2: 6000}}`.
    /// Semantics unconfirmed.
    pub const TIME_WINDOWS: PartKind = PartKind(47);
    /// Protobuf, once per response: field 1 is a short opaque base64 token
    /// that changes between responses. Semantics unconfirmed.
    pub const CONTEXT_TOKEN: PartKind = PartKind(52);
    /// Protobuf, once per response: `{1: 1000, 2: {1: 5000, 2: 32495}}` ms values.
    /// Semantics unconfirmed.
    pub const INTERVAL_POLICY: PartKind = PartKind(53);
    /// Protobuf, first part of every media response: `{1: 1}` status flag.
    pub const STREAM_PROTECTION_STATUS: PartKind = PartKind(58);
    /// Every kind above, ascending.
    pub const OBSERVED: &'static [PartKind] = &[
        PartKind::MEDIA_HEADER,
        PartKind::MEDIA,
        PartKind::MEDIA_END,
        PartKind::NEXT_REQUEST_POLICY,
        PartKind::FORMAT_INITIALIZATION_METADATA,
        PartKind::REDIRECT,
        PartKind::TIME_WINDOWS,
        PartKind::CONTEXT_TOKEN,
        PartKind::INTERVAL_POLICY,
        PartKind::STREAM_PROTECTION_STATUS,
    ];
}

impl fmt::Display for PartKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// One `varint(kind) varint(size) payload[size]` unit of the response stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub kind: PartKind,
    pub payload: Bytes,
}

/// Outcome of a non-blocking decode attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    Part(Part),
    Incomplete,
}

/// Outcome of a blocking read: a part, or the clean end of the stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Next {
    Part(Part),
    End,
}

enum Varint {
    Value(u32, usize),
    Incomplete,
}

enum Header {
    Found { kind: u32, start: usize, end: usize },
    Incomplete,
}

fn to_usize(n: u32) -> usize {
    match usize::try_from(n) {
        Ok(v) => v,
        Err(e) => panic!("u32 does not fit usize on this target: {e}"),
    }
}

fn get_varint(buf: &[u8]) -> Varint {
    let Some(&first) = buf.first() else { return Varint::Incomplete };
    let width = match first {
        0x00..=0x7f => 1,
        0x80..=0xbf => 2,
        0xc0..=0xdf => 3,
        0xe0..=0xef => 4,
        0xf0..=0xff => 5,
    };
    let Some(b) = buf.get(..width) else { return Varint::Incomplete };
    let at = |i: usize| u32::from(b[i]);
    let value = match first {
        0x00..=0x7f => at(0),
        0x80..=0xbf => (at(0) & 0x3f) | at(1) << 6,
        0xc0..=0xdf => (at(0) & 0x1f) | at(1) << 5 | at(2) << 13,
        0xe0..=0xef => (at(0) & 0x0f) | at(1) << 4 | at(2) << 12 | at(3) << 20,
        0xf0..=0xff => u32::from_le_bytes([b[1], b[2], b[3], b[4]]),
    };
    Varint::Value(value, width)
}

fn put_varint(v: u32, out: &mut Vec<u8>) {
    let lo = |x: u32| x.to_le_bytes()[0];
    if v < 1 << 7 {
        out.push(lo(v));
    } else if v < 1 << 14 {
        out.extend([0x80 | (lo(v) & 0x3f), lo(v >> 6)]);
    } else if v < 1 << 21 {
        out.extend([0xc0 | (lo(v) & 0x1f), lo(v >> 5), lo(v >> 13)]);
    } else if v < 1 << 28 {
        out.extend([0xe0 | (lo(v) & 0x0f), lo(v >> 4), lo(v >> 12), lo(v >> 20)]);
    } else {
        out.push(0xf0);
        out.extend(v.to_le_bytes());
    }
}

fn header(buf: &[u8]) -> Header {
    let Varint::Value(kind, a) = get_varint(buf) else { return Header::Incomplete };
    let Varint::Value(size, b) = get_varint(&buf[a..]) else { return Header::Incomplete };
    let start = a + b;
    Header::Found { kind, start, end: start + to_usize(size) }
}

impl Part {
    pub fn new(kind: u32, payload: impl Into<Bytes>) -> Self {
        Self { kind: PartKind(kind), payload: payload.into() }
    }

    /// Decode one part from the front of `buf`, returning it with the number of bytes consumed.
    pub fn decode(buf: &[u8]) -> Result<(Self, usize), PartError> {
        let Header::Found { kind, start, end } = header(buf) else { return Err(PartError::Truncated) };
        let Some(payload) = buf.get(start..end) else { return Err(PartError::Truncated) };
        Ok((Self::new(kind, Bytes::copy_from_slice(payload)), end))
    }

    pub fn encode_into(&self, out: &mut Vec<u8>) -> Result<(), PartError> {
        put_varint(self.kind.0, out);
        put_varint(u32::try_from(self.payload.len())?, out);
        out.extend_from_slice(&self.payload);
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>, PartError> {
        let mut out = Vec::with_capacity(10 + self.payload.len());
        self.encode_into(&mut out)?;
        Ok(out)
    }

    /// Payload parsed as a protobuf message (control parts).
    pub fn message(&self) -> Result<Message, ProtoError> {
        Message::decode(&self.payload)
    }
}

/// Decode a complete in-memory part stream, e.g. a captured response body.
pub fn decode_all(buf: &[u8]) -> Result<Vec<Part>, PartError> {
    let mut parts = Vec::new();
    let mut rest = buf;
    while !rest.is_empty() {
        let (part, n) = Part::decode(rest)?;
        parts.push(part);
        rest = &rest[n..];
    }
    Ok(parts)
}

/// Push-style incremental parser: feed bytes in any chunking, take parts out.
pub struct Decoder {
    buf: BytesMut,
}

impl Decoder {
    const CHUNK: usize = 64 * 1024;

    pub fn new() -> Self {
        Self { buf: BytesMut::with_capacity(Self::CHUNK) }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Spare buffer to read into directly, at least 64 KiB free.
    pub fn space(&mut self) -> &mut BytesMut {
        self.buf.reserve(Self::CHUNK);
        &mut self.buf
    }

    pub fn take(&mut self) -> Frame {
        let Header::Found { kind, start, end } = header(&self.buf[..]) else { return Frame::Incomplete };
        if self.buf.len() < end {
            return Frame::Incomplete;
        }
        self.buf.advance(start);
        Frame::Part(Part::new(kind, self.buf.split_to(end - start).freeze()))
    }

    /// Call at end of input: leftover bytes mean the stream died mid-part.
    pub fn close(&self) -> Result<(), PartError> {
        if self.buf.is_empty() {
            Ok(())
        } else {
            Err(PartError::Truncated)
        }
    }
}

/// Pull-style reader over any `AsyncRead`: HTTP body, file, `&[u8]`.
pub struct PartReader<R> {
    inner: R,
    decoder: Decoder,
}

impl<R: AsyncRead + Unpin> PartReader<R> {
    pub fn new(inner: R) -> Self {
        Self { inner, decoder: Decoder::new() }
    }

    pub fn into_inner(self) -> R {
        self.inner
    }

    pub async fn next(&mut self) -> Result<Next, PartError> {
        loop {
            let Frame::Part(part) = self.decoder.take() else {
                let n = self.inner.read_buf(self.decoder.space()).await?;
                if n == 0 {
                    self.decoder.close()?;
                    return Ok(Next::End);
                }
                continue;
            };
            return Ok(Next::Part(part));
        }
    }
}
