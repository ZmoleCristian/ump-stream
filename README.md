# ump-stream

Universal Media Protocol in pure Rust: protobuf POST requests, chunked
part-stream responses, server-driven adaptive streaming sessions.

No schema, no endpoint constants. Hosts, headers, part kinds and field numbers
are all injected at runtime.

## Wire format

Response body = sequence of parts:

```text
varint(kind) varint(size) payload[size]
```

The header varint is **not** protobuf's. The first byte picks the width, its
low bits hold the low value bits, the rest is little-endian:

| first byte | width | value                                          |
|------------|-------|------------------------------------------------|
| `< 0x80`   | 1     | `b0`                                           |
| `< 0xc0`   | 2     | `b0 & 0x3f \| b1 << 6`                          |
| `< 0xe0`   | 3     | `b0 & 0x1f \| b1 << 5 \| b2 << 13`               |
| `< 0xf0`   | 4     | `b0 & 0x0f \| b1 << 4 \| b2 << 12 \| b3 << 20`    |
| else       | 5     | `u32_le(b1..b5)`                               |

Payloads are either raw media or protobuf control messages. Request bodies are
protobuf.

## API

- `RequestBuilder` — field number → value, nested messages via closure.
- `PartReader<R: AsyncRead>` — `next()` yields `Next::Part(..)` until `Next::End`.
- `Decoder` — push-style parser (`push`, `take`, `close`) for custom transports;
  `decode_all(&[u8])` for in-memory captures.
- `Session` — `send(body)` queues a POST (aborting any live response),
  `next()` yields parts, follows redirects, errors out on the configured
  error kind.
- `proto::Message` — schema-less decoder for control payloads.

Every fallible call returns a typed error (`PartError`, `ProtoError`,
`SessionError`); there are no `Option`s on the public surface.

## Minimal session

```rust,no_run
use ump_stream::{Next, PartKind, Redirect, RequestBuilder, Session, SessionConfig, SessionError};

#[tokio::main]
async fn main() -> Result<(), SessionError> {
    let cfg = SessionConfig {
        url: "https://host/path?params".into(),
        headers: vec![("content-type".into(), "application/x-protobuf".into())],
        redirect: Redirect { kind: PartKind(51), url_field: 1 },
        error: PartKind(52),
    };
    let request = |buffered_ms: u64| {
        RequestBuilder::new()
            .message(1, |client| client.varint(1, buffered_ms))
            .packed(2, [1, 2])
            .build()
    };

    let mut session = Session::new(reqwest::Client::new(), cfg);
    let mut buffered_ms = 0;
    session.send(request(buffered_ms));
    loop {
        match session.next().await? {
            Next::Part(part) => {
                match part.message() {
                    Ok(msg) => println!("control {}: {msg:?}", part.kind),
                    Err(_) => println!("media {} bytes", part.payload.len()),
                }
                buffered_ms += 1000;
            }
            Next::End if session.stats().requests < 3 => session.send(request(buffered_ms)),
            Next::End => break,
        }
    }
    Ok(())
}
```

`examples/session.rs` is the same thing driven by env vars.

## Fixtures

`tests/fixtures/real/` holds captured traffic: `req_*.bin` (protobuf request
bodies) and `resp_*.bin` (de-chunked part streams). It is not published and
not committed (captures carry session tokens); without it those tests skip.
With it, the suite decodes every file, round-trips the requests and drives a
`Session` through a captured redirect:

```text
cargo test --test fixtures -- --nocapture
```

dumps every control payload as a protobuf tree. Part kinds seen in those
captures are documented as `PartKind::*` constants (`PartKind::OBSERVED`
lists them); the test fails if a new kind shows up undocumented.

Captured requests pad length varints (`c0 81 00` for 192). The encoder here
is canonical, so re-encoding is lossless but not byte-identical.

## Law

Source is judged by [lawkeeper](https://crates.io/crates/lawkeeper) (`law.toml`
at the root). The single concession is `DEAD:2`: `///` docs on public items.

## License

0BSD
