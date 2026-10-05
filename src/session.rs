use std::mem;

use crate::config::Config;
use crate::part::{Decoder, Frame, Next, Part, PartError};
use crate::proto::ProtoError;

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("part stream: {0}")]
    Part(#[from] PartError),
    #[error("redirect payload: {0}")]
    Proto(#[from] ProtoError),
    #[error("server error part (kind {})", .0.kind)]
    Server(Part),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stats {
    pub requests: u64,
    pub parts: u64,
    pub bytes: usize,
}

enum State {
    Idle,
    Queued(Vec<u8>),
    Streaming(reqwest::Response),
}

enum Flow {
    More,
    Drained,
}

/// POST → parts → re-POST. Follows redirects and surfaces the configured error kind.
pub struct Session {
    http: reqwest::Client,
    cfg: Config,
    url: String,
    state: State,
    decoder: Decoder,
    stats: Stats,
}

impl Session {
    pub fn new(http: reqwest::Client, cfg: Config) -> Self {
        let url = cfg.url.clone();
        Self {
            http,
            cfg,
            url,
            state: State::Idle,
            decoder: Decoder::new(),
            stats: Stats { requests: 0, parts: 0, bytes: 0 },
        }
    }

    /// Queue the next request body, aborting any response still streaming.
    /// Call it when the client's buffer state changes or after [`Next::End`].
    pub fn send(&mut self, body: Vec<u8>) {
        self.state = State::Queued(body);
        self.decoder = Decoder::new();
    }

    /// Next part. Issues the queued POST first. [`Next::End`] means the current
    /// response is exhausted and nothing is queued.
    pub async fn next(&mut self) -> Result<Next, SessionError> {
        loop {
            let Frame::Part(part) = self.decoder.take() else {
                let Flow::More = self.advance().await? else { return Ok(Next::End) };
                continue;
            };
            return self.handle(part);
        }
    }

    /// Current endpoint, updated by redirects.
    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    async fn advance(&mut self) -> Result<Flow, SessionError> {
        match mem::replace(&mut self.state, State::Idle) {
            State::Idle => Ok(Flow::Drained),
            State::Queued(body) => {
                self.state = State::Streaming(self.post(body).await?);
                self.stats.requests += 1;
                Ok(Flow::More)
            }
            State::Streaming(mut resp) => {
                let Some(chunk) = resp.chunk().await? else {
                    self.decoder.close()?;
                    return Ok(Flow::More);
                };
                self.decoder.push(&chunk);
                self.state = State::Streaming(resp);
                Ok(Flow::More)
            }
        }
    }

    async fn post(&self, body: Vec<u8>) -> Result<reqwest::Response, SessionError> {
        let mut req = self.http.post(&self.url).body(body);
        for (name, value) in &self.cfg.headers {
            req = req.header(name.as_str(), value.as_str());
        }
        Ok(req.send().await?.error_for_status()?)
    }

    fn handle(&mut self, part: Part) -> Result<Next, SessionError> {
        self.stats.parts += 1;
        self.stats.bytes += part.payload.len();
        if part.kind == self.cfg.error {
            return Err(SessionError::Server(part));
        }
        if part.kind == self.cfg.redirect.kind {
            self.url = part.message()?.get(self.cfg.redirect.url_field)?.as_str()?.to_owned();
        }
        Ok(Next::Part(part))
    }
}
