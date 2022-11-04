//! # cdp-rs
//! `cdp-rs` is a Chrome Dev Protocol client, which allows interacting with a browser
//! through the CDP protocol.

use std::net::TcpStream;
use serde::Deserialize;
use serde_json::{json, Value};
use tungstenite::{connect, WebSocket, stream::MaybeTlsStream, error::Error};
use url::Url;

pub type NetworkError = Error;
pub type CdpResult<T> = Result<T, CdpError>;

#[derive(Debug)]
pub enum CdpError {
    CannotConnect,
    InvalidTab,
    NetworkError(NetworkError),
    InvalidResponse,
    NoMessage
}
impl From<Error> for CdpError {
    fn from(error: Error) -> Self {
        match error {
            Error::Utf8 => CdpError::InvalidResponse,
            _ => CdpError::NetworkError(error),
        }
    }
}
impl From<reqwest::Error> for CdpError {
    fn from(_: reqwest::Error) -> Self {
        CdpError::CannotConnect
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
    host: &'static str,
    port: u32,
}
impl CdpClient {
    /// Creates a new client connecting to the default localhost::9222
    pub fn new() -> Self {
        Self::custom("localhost", 9222)
    }

    /// Creates a new client connecting to a custom host and port
    pub fn custom(host: &'static str, port: u32) -> Self {
        Self { host, port }
    }

    /// Returns tabs from the browser instance
    pub fn get_tabs(&self) -> CdpResult<Vec<Tab>> {
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
    /// if let Ok(r) = cdp.send_parms("Target.createTarget", vec![("url", "https://www.google.com")]) {
    ///     let target_id = r["result"]["targetId"];
    ///     let cdp = client.connect_to_target(target_id);
    ///     // Use connection
    /// }
    /// ```
    pub fn connect_to_target(&self, target_id: u32) -> CdpResult<CdpConnection> {
        let ws_url = format!("ws://{}:{}/devtools/page/{}", self.host, self.port, target_id);
        let url = Url::parse(&ws_url).unwrap();
        let (socket, _) = connect(url)?;

        Ok(CdpConnection::new(socket))
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
    pub fn connect_to_tab(&self, tab_index: usize) -> CdpResult<CdpConnection> {
        let tabs = self.get_tabs()?;
        let ws_url = match tabs.get(tab_index) {
            Some(tab) => tab.webSocketDebuggerUrl.clone(),
            None => return Err(CdpError::InvalidTab),
        };

        let url = Url::parse(&ws_url).unwrap();
        let (socket, _) = connect(url)?;

        Ok(CdpConnection::new(socket))
    }
}
impl Default for CdpClient {
    fn default() -> Self {
        Self::new()
    }
}

/// A connection to the browser isntance which can be used to send and recieve messages
/// The connection connection will be closed when the variable is dropped
pub struct CdpConnection {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    message_id: i64,
}
impl CdpConnection {
    fn new(socket: WebSocket<MaybeTlsStream<TcpStream>>) -> Self {
        Self { socket, message_id: 1 }
    }

    /// Sends a message to the browser instance which doesnt require any parameters.
    /// This is the same as calling `send_parms` with an empty Vec
    /// 
    /// # Examples
    /// 
    /// ```
    /// # use cdp_rs::CdpClient;
    /// 
    /// let cdp = CdpClient::new().connect_to_tab(0);
    /// cdp.send("Network.getAllCookies");
    /// ```
    pub fn send(&mut self, method: &'static str) -> CdpResult<Value> {
        self.send_parms::<()>(method, vec!())
    }

    /// Sends a message to the browser instance with the supplied parameters
    /// 
    /// # Examples
    /// 
    /// ```
    /// # use cdp_rs::CdpClient;
    /// 
    /// let cdp = CdpClient::new().connect_to_tab(0);
    /// cdp.send_parms("Network.getCookies", vec![("urls", vec!["https://www.google.com"])]);
    /// ```
    pub fn send_parms<T: Into<Value>>(&mut self, method: &'static str, parms: Vec<(&'static str, T)>) -> CdpResult<Value> {
        let mut map = serde_json::Map::new();
        for p in parms {
            map.insert(p.0.to_string(), p.1.into());
        }

        let data = json!({
            "id": self.message_id,
            "method": method,
            "params": map
        });
        
        self.socket.write_message(tungstenite::Message::Text(data.to_string()))?;
        let result = self.wait_result();
        self.message_id += 1;

        result
    }

    /// Waits for the next message to be recieved. Will block until a event is recieved
    /// 
    /// # Examples
    /// 
    /// ```
    /// # use cdp_rs::CdpClient;
    /// 
    /// let cdp = CdpClient::new().connect_to_tab(0);
    /// let response = cdp.wait_event();
    /// ```
    pub fn wait_message(&mut self) -> CdpResult<Value> {
        if let Ok(msg) = self.socket.read_message() {
            println!("Received: {}", msg);
            let text = msg.into_text()?;

            return match serde_json::from_str::<Value>(&text) {
                Err(_) => Err(CdpError::InvalidResponse),
                Ok(m) => Ok(m)
            }
        }
        Err(CdpError::NoMessage)
    }

    /// Waits for the specified event before returning. Will block until the event is found.
    /// 
    /// # Examples
    /// 
    /// ```
    /// # use cdp_rs::CdpClient;
    /// 
    /// let cdp = CdpClient::new().connect_to_tab(0);
    /// let response = cdp.wait_event("Network.dataReceived");
    /// ```
    pub fn wait_event(&mut self, event: &str) -> CdpResult<Value> {
        while let Ok(m) = self.wait_message() {
            if let Some(method) = m.get("method") {
                if method == event {
                    return Ok(m);
                }
            }
        }
        Err(CdpError::NoMessage)
    }

    fn wait_result(&mut self) -> CdpResult<Value> {
        while let Ok(m) = self.wait_message() {
            if m.get("result").is_some() && m["id"].as_i64().unwrap() == self.message_id {
                return Ok(m);
            }
        }
        Err(CdpError::NoMessage)
    }
}
impl Drop for CdpConnection {
    fn drop(&mut self) {
        if self.socket.close(None).is_ok() {
            // Wait until close message is acknowledged by the other side
            for _ in 0..100 {
                if let Err(Error::ConnectionClosed) = self.socket.write_pending() {
                    break;
                }
            }
        }
    }
}