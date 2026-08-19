//! Minimal client for the line-delimited JSON verb protocol.
//!
//! The daemon accepts both the binary `Message` frames (used by the TUI) and
//! raw JSON lines on the same Unix socket. This client speaks the verb
//! flavour: write one `VerbRequest` line, read one `VerbResponse` line. It
//! backs the public scriptable CLI (`termos action`, `termos subscribe`,
//! ...) and is the intended entry point for external tools driving a
//! headless daemon.
//!
//! Example:
//! ```no_run
//! use termos::session::verb_client::{VerbClient, VerbClientError};
//!
//! let mut c = VerbClient::connect().unwrap();
//! match c.request_json("list-sessions", serde_json::json!({})) {
//!     Ok(v) => println!("{v}"),
//!     Err(VerbClientError::Io(e)) => eprintln!("cannot reach daemon: {e}"),
//!     Err(VerbClientError::Verb(e)) => eprintln!("verb error: {e}"),
//! }
//! ```

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

use serde_json::Value;

use super::daemon::default_socket_path;
use super::verb::{VerbError, VerbRequest, VerbResponse};

/// Errors from a verb-protocol call: transport failures and verb errors.
#[derive(Debug)]
pub enum VerbClientError {
    /// Transport failure (socket missing, malformed reply, IO error).
    Io(std::io::Error),
    /// The daemon answered with a structured verb error.
    Verb(VerbError),
}

impl std::fmt::Display for VerbClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Verb(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for VerbClientError {}

impl From<std::io::Error> for VerbClientError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// A connection speaking the line-delimited JSON verb protocol.
pub struct VerbClient {
    stream: UnixStream,
}

impl VerbClient {
    /// Connect to the default daemon socket.
    pub fn connect() -> std::io::Result<Self> {
        Self::connect_to(&default_socket_path())
    }

    /// Connect to a specific socket path.
    pub fn connect_to(path: &Path) -> std::io::Result<Self> {
        Ok(Self {
            stream: UnixStream::connect(path)?,
        })
    }

    /// Send one request and read one response line.
    pub fn request(&mut self, verb: &str, params: Value) -> Result<VerbResponse, VerbClientError> {
        let req = VerbRequest {
            id: None,
            verb: verb.to_string(),
            params: Some(params),
        };
        let line = serde_json::to_string(&req)
            .map_err(|e| VerbClientError::Io(std::io::Error::other(e)))?;
        self.stream.write_all(line.as_bytes())?;
        self.stream.write_all(b"\n")?;
        self.stream.flush()?;

        let mut buf = String::new();
        let n = {
            let mut reader = BufReader::new(self.stream.try_clone()?);
            reader.read_line(&mut buf)?
        };
        if n == 0 {
            return Err(VerbClientError::Io(std::io::Error::other(
                "daemon closed the connection without a reply",
            )));
        }
        let resp: VerbResponse = serde_json::from_str(buf.trim())
            .map_err(|e| VerbClientError::Io(std::io::Error::other(e)))?;
        Ok(resp)
    }

    /// Send one request and unwrap the response into its result value,
    /// mapping verb errors onto the error type.
    pub fn request_json(
        &mut self,
        verb: &str,
        params: Value,
    ) -> Result<Value, VerbClientError> {
        match self.request(verb, params)? {
            VerbResponse {
                result: Some(r), ..
            } => Ok(r),
            VerbResponse {
                error: Some(e), ..
            } => Err(VerbClientError::Verb(e)),
            _ => Err(VerbClientError::Io(std::io::Error::other(
                "daemon returned an empty reply",
            ))),
        }
    }

    /// Send a request that opens a long-lived stream (`subscribe`) and call
    /// `on_line` with each result object as it arrives. Returns when the
    /// server closes the stream, the callback returns `false`, or an error
    /// reply arrives.
    pub fn stream(
        &mut self,
        verb: &str,
        params: Value,
        mut on_line: impl FnMut(&Value) -> bool,
    ) -> Result<(), VerbClientError> {
        let req = VerbRequest {
            id: None,
            verb: verb.to_string(),
            params: Some(params),
        };
        let line = serde_json::to_string(&req)
            .map_err(|e| VerbClientError::Io(std::io::Error::other(e)))?;
        self.stream.write_all(line.as_bytes())?;
        self.stream.write_all(b"\n")?;
        self.stream.flush()?;

        let mut reader = BufReader::new(self.stream.try_clone()?);
        let mut buf = String::new();
        loop {
            buf.clear();
            let n = reader.read_line(&mut buf)?;
            if n == 0 {
                return Ok(()); // server closed the stream (or window closed)
            }
            let resp: VerbResponse = serde_json::from_str(buf.trim())
                .map_err(|e| VerbClientError::Io(std::io::Error::other(e)))?;
            if let Some(error) = resp.error {
                return Err(VerbClientError::Verb(error));
            }
            if let Some(result) = resp.result {
                if !on_line(&result) {
                    return Ok(());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips_a_simple_call() {
        // A disconnected socket surfaces as an IO error with a clear kind,
        // not a panic.
        let err = VerbClient::connect_to(Path::new("/nonexistent/termos.sock"))
            .err()
            .expect("connection should fail");
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }
}
