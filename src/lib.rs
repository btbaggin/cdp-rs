//! # cdp-rs
//! `cdp-rs` is a Chrome Dev Protocol client, which allows interacting with a browser
//! through the CDP protocol.

use std::{net::TcpStream, time::{Duration, Instant}};
use serde::Deserialize;
use serde_json::{json, Value};
use tungstenite::{client, WebSocket, error::Error, handshake::HandshakeError};
use url::Url;

/// Represents an error that occurred while making a network request
pub type NetworkError = Error;
/// Parameter type to the send method
pub type MessageParameter = Value;

#[derive(Debug)]
pub enum ClientError {
    /// There was an issue connecting to the browser instance.
    /// This could be because an instance was not launched with remote-debugging-port set
    CannotConnect,
    /// The tab that was attempted to be connected does not exist
    InvalidTab
}
impl From<reqwest::Error> for ClientError {
    fn from(_: reqwest::Error) -> Self {
        ClientError::CannotConnect
    }
}

#[derive(Debug)]
pub enum MessageError {
    /// An error occurred while sending or recieving a message
    NetworkError(NetworkError),
    /// The sent request was invalid
    InvalidRequest(Value),
    /// A response was recieved from the CDP connection that was not properly formatted
    InvalidResponse,
    /// Returned when calling a `wait` method on the CDP connection but no messages are available
    NoMessage
}
impl From<Error> for MessageError {
    fn from(error: Error) -> Self {
        match error {
            Error::Utf8 => MessageError::InvalidResponse,
            _ => MessageError::NetworkError(error),
        }
    }
}

/// Information about a tab as retrieved from the CDP connection
#[allow(non_snake_case, dead_code)]
#[derive(Deserialize)]
pub struct Tab {
    description: String,
    devtoolsFrontendUrl: String,
    id: String,
    title: String,
    r#type: String,
    url: String,
    webSocketDebuggerUrl: String
}

/// Client which stores the information about which host and port to connect to.
/// The only purpose of this class is to get a `CdpConnection` which can be used
/// to interact with the browser instance
pub struct CdpClient {
    host: String,
    port: u16,
}
impl CdpClient {
    /// Creates a new client connecting to the default localhost::9222
    pub fn new() -> Self {
        Self::custom("localhost", 9222)
    }

    /// Creates a new client connecting to a custom host and port
    pub fn custom(host: &str, port: u16) -> Self {
        Self { host: host.to_string(), port }
    }

    /// Returns tabs from the browser instance
    pub fn get_tabs(&self) -> Result<Vec<Tab>, ClientError> {
        let tabs = reqwest::blocking::get(format!("http://{}:{}/json", self.host, self.port))?
            .json::<Vec<Tab>>()?;
        Ok(tabs)
    }

    /// Creates a `CdpConnection` to a specifed targetId
    /// 
    /// # Examples
    /// 
    /// ```
    /// # use cdp_rs::CdpClient;
    /// 
    /// let client = CdpClient::new();
    /// let cdp = client.connect_to_tab(0);
    /// if let Ok(r) = cdp.send("Target.createTarget", parms!("url", "https://www.google.com")) {
    ///     let target_id = r["result"]["targetId"];
    ///     let cdp = client.connect_to_target(target_id);
    ///     // Use connection
    /// }
    /// ```
    pub fn connect_to_target(&self, target_id: &str) -> Result<CdpConnection, ClientError> {
        let ws_url = format!("ws://{}:{}/devtools/page/{}", self.host, self.port, target_id);
        CdpClient::make_connection(&ws_url, self.port)
    }

    /// Creates a `CdpConnection` to a the tab specified by index
    /// 
    /// # Examples
    /// 
    /// ```
    /// # use cdp_rs::CdpClient;
    /// 
    /// let cdp = CdpClient::new().connect_to_tab(0);
    /// // Use connection
    /// ```
    pub fn connect_to_tab(&self, tab_index: usize) -> Result<CdpConnection, ClientError> {
        let tabs = self.get_tabs()?;
        let ws_url = match tabs.get(tab_index) {
            Some(tab) => tab.webSocketDebuggerUrl.clone(),
            None => return Err(ClientError::InvalidTab),
        };

        CdpClient::make_connection(&ws_url, self.port)
    }

    fn make_connection(url: &str, port: u16) -> Result<CdpConnection, ClientError> {
        let url = Url::parse(&url).unwrap();
        let mut addrs = url.socket_addrs(|| Some(port)).unwrap();
        // Sort addresses by IPv4 first since IPv6 usually doesn't connect
        addrs.sort();

        for addr in addrs {
            if let Ok(stream) = TcpStream::connect(addr) {
                stream.set_nonblocking(true).unwrap();
                
                let mut result = client(url.clone(), stream);
                loop {
                    match result {
                        Ok((socket, _)) => return Ok(CdpConnection::new(socket)),
                        Err(HandshakeError::Failure(_)) => return Err(ClientError::CannotConnect),
                        Err(HandshakeError::Interrupted(mid)) => result = mid.handshake(),
                    }
                }
            }
        }
        

        Err(ClientError::CannotConnect)
    }

}
impl Default for CdpClient {
    fn default() -> Self {
        Self::new()
    }
}

#[macro_export]
macro_rules! parms {
    ($($name:literal, $value:expr),*) => {{
        vec![$(($name, cdp_rs::MessageParameter::from($value))),*]
    }};
}

/// A connection to the browser isntance which can be used to send and recieve messages
/// The connection connection will be closed when the variable is dropped
pub struct CdpConnection {
    socket: WebSocket<TcpStream>,
    message_id: i64,
}
impl CdpConnection {
    fn new(socket: WebSocket<TcpStream>) -> Self {
        Self { socket, message_id: 1 }
    }

    /// Sends a message to the browser instance with the supplied parameters
    /// 
    /// # Examples
    /// 
    /// ```
    /// # use cdp_rs::CdpClient;
    /// 
    /// let mut cdp = CdpClient::new().connect_to_tab(0);
    /// cdp.send("Network.getCookies", parms!("urls", vec!["https://www.google.com"]));
    /// ```
    pub fn send(&mut self, method: &'static str, parms: Vec<(&'static str, MessageParameter)>) -> Result<Value, MessageError> {
        let message_id = self.message_id;
        let mut map = serde_json::Map::new();
        for p in parms {
            map.insert(p.0.to_string(), p.1);
        }

        let data = json!({
            "id": self.message_id,
            "method": method,
            "params": map
        });
        
        self.message_id += 1;
        self.socket.write_message(tungstenite::Message::Text(data.to_string()))?;
        let result = self.wait_for(None, |m| {
            (m.get("error").is_some() || m.get("result").is_some()) &&
            m["id"].as_i64().unwrap() == message_id
        });

        // Check if there was an error response
        if let Ok(r) = &result {
            if r.get("error").is_some() { return Err(MessageError::InvalidRequest(r.clone())) }
        }
        result
    }

    /// Waits for the next message to be recieved.
    /// Will return NoMessage if there are no messages available
    /// 
    /// # Examples
    /// 
    /// ```
    /// # use cdp_rs::CdpClient;
    /// 
    /// let mut cdp = CdpClient::new().connect_to_tab(0);
    /// let response = cdp.wait_message();
    /// ```
    pub fn wait_message(&mut self) -> Result<Value, MessageError> {
        if let Ok(msg) = self.socket.read_message() {
            let text = msg.into_text()?;

            return match serde_json::from_str::<Value>(&text) {
                Err(_) => Err(MessageError::InvalidResponse),
                Ok(m) => Ok(m)
            }
        }
        Err(MessageError::NoMessage)
    }

    /// Waits for the specified event before returning. Will block until the event is found.
    /// 
    /// # Examples
    /// 
    /// ```
    /// # use cdp_rs::CdpClient;
    /// 
    /// let mut cdp = CdpClient::new().connect_to_tab(0);
    /// cdp.send("Network.enable", parms!()).unwrap();
    /// let response = cdp.wait_event("Network.dataReceived", None);
    /// ```
    pub fn wait_event(&mut self, event: &str, timeout: Option<Duration>) -> Result<Value, MessageError> {
        self.wait_for(timeout, |m| {
            if let Some(method) = m.get("method") {
                if method == event {
                    return true
                }
            }
            return false
        })
    }

    /// Waits for a user defined condition to be true before returning.
    /// 
    /// # Examples
    /// 
    /// ```
    /// # use cdp_rs::CdpClient;
    /// 
    /// let mut cdp = CdpClient::new().connect_to_tab(0);
    /// let response = cdp.wait_for(|m| { m.get("result").is_some() });
    /// ```
    pub fn wait_for<F>(&mut self, timeout: Option<Duration>, f: F) -> Result<Value, MessageError>
        where F: Fn(&Value) -> bool {

        let timeout = match timeout {
            Some(t) => t,
            None => Duration::from_secs(300),
        };

        let now = Instant::now();
        while Instant::now() - now < timeout {
            let m = self.wait_message();
            match m {
                Ok(m) => if f(&m) { return Ok(m) },
                Err(MessageError::NoMessage) => {},
                _ => { break; }
            }
        }
        Err(MessageError::NoMessage)
    }

}
impl Drop for CdpConnection {
    fn drop(&mut self) {
        if self.socket.close(None).is_ok() {
            // Wait until close message is acknowledged by the other side
            for _ in 0..100 {
                if matches!(self.socket.read_message(), Err(Error::ConnectionClosed) | Err(Error::AlreadyClosed)) {
                    break;
                }
            }
        }
    }
}
