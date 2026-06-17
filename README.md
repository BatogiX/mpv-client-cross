# MPV plugins in Rust

Bindings for libmpv client API that allow you to create plugins for MPV in Rust.

> ⚠️ **About this Fork:**
> This is a maintained fork of the original [TheCactusVert/mpv-client](https://github.com/TheCactusVert/mpv-client). 
> 
> **Key improvements in this fork:**
> * **Windows Support:** via MPV_CPLUGIN_DYNAMIC_SYM [linkage-to-libmpv](https://mpv.io/manual/stable/#linkage-to-libmpv).
> * **Android Support:** via MPV_CPLUGIN_DYNAMIC_SYM [linkage-to-libmpv](https://mpv.io/manual/stable/#linkage-to-libmpv).
> * **No LLVM/Clang required:** Uses **pregenerated bindings** by default, meaning you don't need `bindgen` or a local LLVM installation during `cargo build`.
> * **Ergonomic `#[mpv_client::main]` Macro:** Removes C-FFI boilerplate by wrapping your main function and automatically generating the underlying `mpv_open_cplugin` entry point.
> * **Built-in Configuration Parsing:** Includes a seamless `read_options()` helper with `serde` support to automatically merge and parse options from both configuration files (`~~/script-opts/`) and MPV command-line arguments.
> * **Logging:** The macro or ```mp.initialize_logging``` method automatically initializes a custom global `log` implementation (`MpvLogger`) that prints colored messages to stderr and seamlessly matches/synchronizes its timestamps with MPV's native `--log-file` timeline.

## Example

### Init plugin

Here is an example for your `Cargo.toml`:

```toml
[package]
name = "mpv-plugin"
version = "0.1.0"
edition = "2024"

[lib]
name = "mpv_plugin"
crate-type = ["cdylib"]

[dependencies]
mpv-client = { version = "4.0", package = "mpv-client-cross" }
```

And then the code `src/lib.rs`:

```rust
use mpv_client::{mpv_handle, Event, Handle};

#[no_mangle]
unsafe extern "C" fn mpv_open_cplugin(handle: *mut mpv_handle) -> i32 {
  let (mp, event_token) = unsafe { Handle::from_ptr(handle) };
  
  println!("Hello world from Rust plugin {}!", mp.name());
  
  loop {
    match mp.wait_event(&mut event_token, -1.) {
      Event::Shutdown => break,
      event => println!("Got event: {}", event),
    }
  }

  0
}
```

or

```toml
mpv-client = { version = "4.0", package = "mpv-client-cross", features = ["macros"] }
```

```rust
use mpv_client::{Event, Handle, EventQueueToken};

#[mpv_client::main]
fn main(mp: &Handle, mut event_token: EventQueueToken) -> i32 {
  log::info!("Hello world from Rust plugin {}!", mp.name());
  
  loop {
    match mp.wait_event(&mut event_token, -1.) {
      Event::Shutdown => break,
      event => log::info!("Got event: {}", event),
    }
  }

  0
}
```

⚠️ Important Warning:
#[mpv_client::main] macro automatically initializes global logger internally. If your code already sets up a third-party logger (for example, via env_logger::init(), log4rs, or similar crates), make sure to remove or disable its initialization. Attempting to initialize a global logger twice will result in a runtime error (application panic).

### Read config options

First, define your configuration structure using serde deserialization. It is highly recommended to use #[serde(default)] so the plugin can gracefully fall back to default values if options are missing from the configuration files.

```rust
use serde::Deserialize;
use mpv_client::{Event, Handle, EventQueueToken};

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct PluginConfig {
    /// Enable or disable the plugin features
    pub enabled: bool,

    /// Maximum number of items to process/display
    pub max_items: u32,

    /// Optional API token or path to a custom asset
    pub auth_token: Option<String>,

    /// Select the visual layout style
    pub layout: LayoutMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LayoutMode {
    #[default]
    Default,
    Compact,
    Expanded,
}
```

You can call read_options() directly on your Handle instance. The method automatically infers the plugin name and looks up settings from both configuration files and the command line.

```rust
#[mpv_client::main]
fn main(mp: &Handle, mut event_token: EventQueueToken) -> i32 {
    // Automatically reads options and falls back to defaults if not found
    let config: PluginConfig = mp.read_options();
    
    println!("Plugin initialized with config: {:#?}", config);
    
    loop {
        match mp.wait_event(&mut event_token, -1.) {
            Event::Shutdown => break,
            event => log::info!("Got event: {}", event),
        }
    }

    0
}
```

You can find more examples in [`C`](https://github.com/mpv-player/mpv-examples/tree/master/cplugins) and [`Rust`](https://github.com/TheCactusVert/mpv-sponsorblock).
