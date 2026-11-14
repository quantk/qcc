//! The daemon API.
//!
//! Line-delimited JSON-RPC 2.0 over a unix socket. Every method is a type
//! implementing [`Method`], so the daemon and its clients share one definition of
//! names, parameters and results.

pub mod jsonrpc;
pub mod methods;
pub mod paths;

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Bumped on breaking changes to existing methods. Additions do not bump it.
pub const PROTOCOL_VERSION: u32 = 1;

/// A request method: its wire name and the shapes it carries.
pub trait Method {
    const NAME: &'static str;
    type Params: Serialize + DeserializeOwned;
    type Result: Serialize + DeserializeOwned;
}
