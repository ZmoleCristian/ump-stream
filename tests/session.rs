mod common;

use tokio::net::TcpListener;
use ump_stream::{decode_all, Next, Part, PartKind, Redirect, RequestBuilder, Session, SessionConfig, SessionError};

fn config(url: String, redirect: Redirect, error: PartKind) -> SessionConfig {
    SessionConfig {
        url,
        headers: vec![("content-type".into(), "application/x-protobuf".into())],
        redirect,
        error,
    }
}

#[tokio::test]
async fn follows_redirect_reposts_and_surfaces_error() {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());

    let redirect = Part::new(51, RequestBuilder::new().string(1, format!("{base}/b")).build());
    let redirect_len = redirect.payload.len();
    let r1 = common::concat(&[Part::new(20, b"init".as_slice()), Part::new(21, vec![0xaa; 500]), redirect]);
    let r2 = common::concat(&[Part::new(22, b"".as_slice())]);
    let r3 = common::concat(&[Part::new(52, RequestBuilder::new().varint(1, 403).build())]);
    let hits = common::spawn(l, vec![r1, r2, r3]);

    let cfg = config(format!("{base}/a"), Redirect { kind: PartKind(51), url_field: 1 }, PartKind(52));
    let mut s = Session::new(reqwest::Client::new(), cfg);
    assert_eq!(s.next().await.unwrap(), Next::End, "nothing queued yet");

    s.send(RequestBuilder::new().varint(1, 1).build());
    let mut kinds = Vec::new();
    while let Next::Part(p) = s.next().await.unwrap() {
        kinds.push(p.kind.0);
    }
    assert_eq!(kinds, [20, 21, 51]);
    assert_eq!(s.url(), format!("{base}/b"));
    assert_eq!(s.next().await.unwrap(), Next::End, "idle without a queued body");

    s.send(RequestBuilder::new().varint(1, 2).build());
    assert!(matches!(s.next().await.unwrap(), Next::Part(Part { kind: PartKind(22), .. })));
    assert_eq!(s.next().await.unwrap(), Next::End);

    s.send(Vec::new());
    match s.next().await {
        Err(SessionError::Server(p)) => assert_eq!(p.message().unwrap().get(1).unwrap().as_u64().unwrap(), 403),
        other => panic!("expected server error, got {other:?}"),
    }

    let hits = hits.lock().unwrap();
    assert_eq!(hits.iter().map(|h| h.path.as_str()).collect::<Vec<_>>(), ["/a", "/b", "/b"]);
    assert_eq!(hits[0].body, [0x08, 0x01]);
    assert_eq!(hits[1].body, [0x08, 0x02]);
    let st = s.stats();
    assert_eq!((st.requests, st.parts, st.bytes), (3, 5, 4 + 500 + redirect_len + 3));
}

#[tokio::test]
async fn real_resp_001_redirects_session() {
    let Some((_, data)) = common::real("resp_001").pop() else { return };
    let Some((_, body)) = common::real("req_001").pop() else { return };
    let expected: Vec<Part> = decode_all(&data).unwrap();
    let target = expected
        .iter()
        .find(|p| p.kind == PartKind::REDIRECT)
        .map(|p| p.message().unwrap().get(1).unwrap().as_str().unwrap().to_owned())
        .expect("resp_001 has no redirect part");

    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let hits = common::spawn(l, vec![data]);

    let cfg = config(format!("{base}/videoplayback"), Redirect { kind: PartKind::REDIRECT, url_field: 1 }, PartKind(u32::MAX));
    let mut s = Session::new(reqwest::Client::new(), cfg);
    s.send(body.clone());

    let mut got = Vec::new();
    while let Next::Part(p) = s.next().await.unwrap() {
        got.push(p);
    }
    assert_eq!(got, expected);
    assert_eq!(s.url(), target);
    assert_eq!(hits.lock().unwrap()[0].body, body);
}
