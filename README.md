# MPV plugins in Rust

Bindings for libmpv client API that allow you to create plugins for MPV in Rust.

> ⚠️ **About this Fork:**
> This is a maintained fork of the original [TheCactusVert/mpv-client](https://github.com/TheCactusVert/mpv-client). 
> 
> **Key improvements in this fork:**
> * **Windows Support:** via MPV_CPLUGIN_DYNAMIC_SYM [linkage-to-libmpv](https://mpv.io/manual/stable/#linkage-to-libmpv).
> * **Android Support:** via MPV_CPLUGIN_DYNAMIC_SYM [linkage-to-libmpv](https://mpv.io/manual/stable/#linkage-to-libmpv).
> * **No LLVM/Clang required:** Uses **pregenerated bindings** by default, meaning you don't need `bindgen` or a local LLVM installation during `cargo build`.

## Example

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
mpv-client = { version = "2.0", package = "mpv-client-cross" }
```

And then the code `src/lib.rs`:

```rust
use mpv_client::{mpv_handle, Event, Handle};

#[no_mangle]
extern "C" fn mpv_open_cplugin(handle: *mut mpv_handle) -> std::os::raw::c_int {
  let client = Handle::from_ptr(handle);
  
  println!("Hello world from Rust plugin {}!", client.name());
  
  loop {
    match client.wait_event(-1.) {
      Event::Shutdown => { return 0; },
      event => { println!("Got event: {}", event); },
    }
  }
}
```

or

```toml
mpv-client = { version = "2.0", package = "mpv-client-cross", features = ["macros"] }
```

```rust
use mpv_client::{Event, Handle};

#[mpv_client::main]
fn main(client: &mut Handle) -> i32 {
  println!("Hello world from Rust plugin {}!", client.name());
  
  loop {
    match client.wait_event(-1.) {
      Event::Shutdown => { return 0; },
      event => { println!("Got event: {}", event); },
    }
  }
}
```

You can find more examples in [`C`](https://github.com/mpv-player/mpv-examples/tree/master/cplugins) and [`Rust`](https://github.com/TheCactusVert/mpv-sponsorblock).
