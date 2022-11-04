use std::net::TcpStream;
use serde::Deserialize;
use serde_json::{json, Value};
use tungstenite::{connect, WebSocket, stream::MaybeTlsStream, error::Error};
use url::Url;

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

pub struct CdpClient {
    host: &'static str,
    port: u32,
}
impl CdpClient {
    pub fn new() -> Self {
        Self::custom("localhost", 9222)
    }
    pub fn custom(host: &'static str, port: u32) -> Self {
        Self { host, port }
    }

    pub fn get_tabs(&self) -> Vec<Tab> {
        reqwest::blocking::get(format!("http://{}:{}/json", self.host, self.port)).unwrap()
            .json::<Vec<Tab>>().unwrap()
    }

    pub fn connect_to_target(&self, target_id: u32) -> CdpConnection {
        let ws_url = format!("ws://{}:{}/devtools/page/{}", self.host, self.port, target_id);
        let url = Url::parse(&ws_url).unwrap();
        let (socket, _) = connect(url).expect("Can't connect");

        CdpConnection { socket, message_id: 1 }
    }

    pub fn connect_to_tab(&self, tab_index: usize) -> CdpConnection {
        let tabs = self.get_tabs();
        let ws_url = tabs[tab_index].webSocketDebuggerUrl.clone();
        let url = Url::parse(&ws_url).unwrap();
        let (socket, _) = connect(url).expect("Can't connect");

        CdpConnection { socket, message_id: 1 }
    }
}

pub struct CdpConnection {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    pub message_id: i64,
}
impl CdpConnection {
    pub fn send(&mut self, method: &'static str) -> Result<Value, Error> {
        let data = json!({
            "id": self.message_id,
            "method": method,
            "params": {}
        });
        self.send_raw(data)
    }

    pub fn send_parms<T: Into<Value>>(&mut self, method: &'static str, parms: Vec<(&'static str, T)>) -> Result<Value, Error> {
        let mut map = serde_json::Map::new();
        for p in parms {
            map.insert(p.0.to_string(), p.1.into());
        }

        let data = json!({
            "id": self.message_id,
            "method": method,
            "params": map
        });
        
       self.send_raw(data)
    }

    fn send_raw(&mut self, data: Value) -> Result<Value, Error> {
        self.socket.write_message(tungstenite::Message::Text(data.to_string()))?;
        let result = self.wait_result();
        self.message_id += 1;

        result
    }

    pub fn wait_message(&mut self) -> Result<Value, Error> {
        if let Ok(msg) = self.socket.read_message() {
            println!("Received: {}", msg);
            let text = msg.into_text()?;

            // It's probably safe to assume if we are getting a response back
            // that it's valid JSON
            let m: Value = serde_json::from_str(&text).unwrap();
            if m.get("result").is_some() && m["id"].as_i64().unwrap() == self.message_id {
                return Ok(m);
            }
        }
        Err(Error::Utf8) //TODO fix
    }

    pub fn wait_event(&mut self, event: &str) -> Result<Value, Error> {
        let m = self.wait_message()?;
        if let Some(method) = m.get("method") {
            if method == event {
                return Ok(m);
            }
        }
        Err(Error::Utf8) //TODO fix
    }

    fn wait_result(&mut self) -> Result<Value, Error> {
        loop {
            let m = self.wait_message()?;
            if m.get("result").is_some() && m["id"].as_i64().unwrap() == self.message_id {
                return Ok(m);
            }
        }
    }
}