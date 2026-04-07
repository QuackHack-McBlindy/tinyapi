# **tinyapi**

 
[![Sponsors](https://img.shields.io/github/sponsors/QuackHack-McBlindy?logo=githubsponsors&label=Sponsor&style=flat&labelColor=ff1493&logoColor=fff&color=rgba(234,74,170,0.5) "")](https://github.com/sponsors/QuackHack-McBlindy) [![Buy Me a Coffee](https://img.shields.io/badge/Buy%20Me%20a%20Coffee-Sponsor?style=flat&logo=buymeacoffee&logoColor=fff&labelColor=ff1493&color=ff1493)](https://buymeacoffee.com/quackhackmcblindy)


`tinyapi` is a crate designed for bare metal and `no_std` Rust async projects.   
Its sole purpose is to make it **stupid easy** to create an embedded API.  
  
`tinyapi` depends on `embassy-net` for network, `embassy-executor` for async task handling.  
It's also assumed you already have network/WiFi configured properly and that an alloc is present.   
  
**User defines:**  
  - endpoint  
  - function to execute or file to serve  
  - response  

The embassy-executor `web_server_task` can then be started in your main loop by passing your network stack.   


## **Installation**

  
Add **tinyapi** as a dependency in `Cargo.toml`.

```toml
[dependencies]
tinyapi = "0.1.1"
```
  

<br>


## **Example usage**

```rust
//! Example for ESP32‑S3 with `esp‑alloc` and `embassy‑net`.
#![no_std]
#![no_main]

extern crate alloc;
use alloc::format;
use defmt::info;
use embassy_executor::Spawner;
use esp_alloc as _; // global allocator
use tinyapi::{register_route, Response, web_server_task};

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // ... WiFi setup, get `stack: &'static Stack<'static>` ...

    // Register endpoints
    // Serve an embedded index.html (place file in crate root)
    register_route("/", |_req| {
        Response::html(include_str!("index.html"))
    }).await;

    // Inline HTML response
    register_route("/hello_world", |_req| {
        Response::html("<h1>Hello from ESP32!</h1>")
    }).await;

    // Path parameter example
    register_route("/led/{state}", |req| {
        let state = req.param("state").unwrap_or("?");
        info!("Setting LED to {}", state);
        Response::text(&format!("LED is now {}", state))
    }).await;

    // Start server
    spawner.spawn(web_server_task(stack)).unwrap();

    loop { /* other tasks */ }
}
```


<br>

## **Lisence**

**MIT**  
<br>
Contributions are welcomed.


<br><br>

## ☕

[![Sponsors](https://img.shields.io/github/sponsors/QuackHack-McBlindy?logo=githubsponsors&label=Sponsor&style=flat&labelColor=ff1493&logoColor=fff&color=rgba(234,74,170,0.5) "")](https://github.com/sponsors/QuackHack-McBlindy) [![Buy Me a Coffee](https://img.shields.io/badge/Buy%20Me%20a%20Coffee-Sponsor?style=flat&logo=buymeacoffee&logoColor=fff&labelColor=ff1493&color=ff1493)](https://buymeacoffee.com/quackhackmcblindy)
> Like my work?   
> Buy me a coffee, or become a sponsor.  
> Thanks for supporting open source!    

<a href="https://www.buymeacoffee.com/quackhackmcblindy" target="_blank"><img src="https://cdn.buymeacoffee.com/buttons/v2/default-yellow.png" alt="Buy Me A Coffee" style="height: 60px !important;width: 217px !important;" ></a>

