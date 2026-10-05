#![doc = include_str!("../README.md")]
#![deny(unused_must_use)]
#![deny(for_loops_over_fallibles)]
#![deny(dead_code)]
#![deny(unused_variables)]
#![deny(unused_assignments)]

pub mod config;
pub mod part;
pub mod proto;
pub mod session;

pub use config::{Config as SessionConfig, Redirect};
pub use part::{decode_all, Decoder, Frame, Next, Part, PartError, PartKind, PartReader};
pub use proto::{Field, Message, ProtoError, RequestBuilder, Value};
pub use session::{Session, SessionError, Stats};
