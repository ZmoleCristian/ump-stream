use crate::part::PartKind;

/// Which part carries a new endpoint and which protobuf field inside it holds the URL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Redirect {
    pub kind: PartKind,
    pub url_field: u32,
}

/// Everything endpoint-specific a [`crate::Session`] needs. Nothing here is baked into the crate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// Initial POST target; replaced in-session by redirects.
    pub url: String,
    /// Sent verbatim on every POST (content type, user agent, ...).
    pub headers: Vec<(String, String)>,
    pub redirect: Redirect,
    /// Part kind that ends the session with [`crate::SessionError::Server`].
    pub error: PartKind,
}
