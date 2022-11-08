# cdp-rs
Allows interacting with a browser using [Chrome Dev Protocol](https://chromedevtools.github.io/devtools-protocol/).
To use this you must launch an instance of Chrome with remote-debugging-port
```
chrome.exe --remote-debugging-port=9222
```

### Examples
```rust
use cdp_rs::CdpClient;

// Connect to the first tab of your open Chrome instance
let mut cdp = CdpClient::new().connect_to_tab(0)

// Send a message so we can recieve DOM events
cdp.send("DOM.enable", parms!());
while Ok(m) = cdp.wait_message() {
    // Print out all messages recieved
    print!("Recieved: {}", m)
}
```

```rust
use cdp_rs::CdpClient;

// Connect to first tab to a chrome instance running on a non-default remote-debugging-port
let mut cdp = CdpClient::custom("localhost", 9000).connect_to_tab(0);
// Send message with parameters and recieve the response
let cookies = cdp.send("Network.getCookies", parms!("urls", vec!["https://www.google.com"]))?;
// Check cookies
```