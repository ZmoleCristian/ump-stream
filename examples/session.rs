//! Minimal session driven by env vars, so nothing endpoint-specific is in the code:
//!   UMP_URL       endpoint
//!   UMP_HEADERS   "k=v;k=v"
//!   UMP_REDIRECT  "kind:url_field"
//!   UMP_ERROR     "kind"
//! The request fields below are placeholders: use the numbers your server expects.

use ump_stream::{Next, PartKind, Redirect, RequestBuilder, Session, SessionConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let env = |k: &str| std::env::var(k).map_err(|e| format!("{k}: {e}"));
    let (kind, field) = env("UMP_REDIRECT")?.split_once(':').map(|(k, f)| (k.to_owned(), f.to_owned())).ok_or("UMP_REDIRECT must be kind:field")?;
    let cfg = SessionConfig {
        url: env("UMP_URL")?,
        headers: env("UMP_HEADERS")
            .unwrap_or_default()
            .split(';')
            .filter_map(|kv| kv.split_once('='))
            .map(|(k, v)| (k.into(), v.into()))
            .collect(),
        redirect: Redirect { kind: PartKind(kind.parse()?), url_field: field.parse()? },
        error: PartKind(env("UMP_ERROR")?.parse()?),
    };

    let request = |buffered: u64| {
        RequestBuilder::new()
            .message(1, |client| client.varint(1, buffered).bool(2, true))
            .packed(2, [1, 2])
            .build()
    };

    let mut session = Session::new(reqwest::Client::new(), cfg);
    let mut buffered = 0;
    session.send(request(buffered));
    loop {
        match session.next().await? {
            Next::Part(part) => {
                println!("part kind={} size={}", part.kind, part.payload.len());
                buffered += u64::try_from(part.payload.len())?;
            }
            Next::End if session.stats().requests < 3 => session.send(request(buffered)),
            Next::End => break,
        }
    }
    println!("{:?}", session.stats());
    Ok(())
}
