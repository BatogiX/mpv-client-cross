use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemFn, parse_macro_input};

#[proc_macro_attribute]
pub fn main(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input_fn = parse_macro_input!(item as ItemFn);
    let fn_name = &input_fn.sig.ident;

    let expanded = quote! {
        #input_fn

        #[unsafe(no_mangle)]
        unsafe extern "C" fn mpv_open_cplugin(handle: *mut ::mpv_client::mpv_handle) -> i32 {
            let (mp, event_token) = unsafe { ::mpv_client::Handle::from_ptr(handle) };
            mp.init_logger().expect("logger is already set");
            #fn_name(mp, event_token)
        }
    };

    TokenStream::from(expanded)
}
