use std::env;
use std::path::PathBuf;

const DYN_SYM_TARGETS: [&str; 2] = ["windows", "android"];

fn main() {
    let out_path = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    let target_os = env::var("CARGO_CFG_TARGET_OS").expect("CARGO_CFG_TARGET_OS not set");

    #[cfg(feature = "bindgen")]
    {
        let bindings = bindgen::Builder::default()
            .header("include/client.h")
            .no_copy("mpv_handle")
            .clang_arg("-target")
            .clang_arg("x86_64-unknown-linux-gnu")
            .generate()
            .expect("Unable to generate bindings");

        let out_file = out_path.join("bindings.rs");
        bindings.write_to_file(&out_file).expect("Couldn't write bindings!");

        if DYN_SYM_TARGETS.contains(&target_os.as_str()) {
            rewrite_dynamic_sym_bindings(&out_file);
        }
    }

    #[cfg(not(feature = "bindgen"))]
    {
        let crate_path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"));

        let src = if DYN_SYM_TARGETS.contains(&target_os.as_str()) {
            crate_path.join("pregenerated_bindings_dyn_sym.rs")
        } else {
            crate_path.join("pregenerated_bindings.rs")
        };

        std::fs::copy(src, out_path.join("bindings.rs")).expect("Couldn't find pregenerated bindings!");
    }
}

#[cfg(feature = "bindgen")]
fn rewrite_dynamic_sym_bindings(path: &std::path::Path) {
    use std::fs;

    use proc_macro2::TokenStream;
    use quote::quote;
    use syn::{ForeignItem, Item};

    let src = fs::read_to_string(path).expect("read bindings");
    let ast = syn::parse_file(&src).expect("parse bindings");

    let mut out_items: Vec<TokenStream> = Vec::new();
    let mut wrappers: Vec<TokenStream> = Vec::new();

    for item in &ast.items {
        let Item::ForeignMod(fm) = item else {
            out_items.push(quote! { #item });
            continue;
        };

        let is_c = fm.abi.name.as_ref().is_some_and(|s| s.value() == "C");
        if !is_c {
            out_items.push(quote! { #item });
            continue;
        }

        let mut leftover: Vec<&ForeignItem> = Vec::new();

        for fi in &fm.items {
            let ForeignItem::Fn(f) = fi else {
                leftover.push(fi);
                continue;
            };

            if f.sig.variadic.is_some() {
                leftover.push(fi);
                continue;
            }

            let fn_ident = &f.sig.ident;
            let pfn_ident = syn::Ident::new(&format!("pfn_{fn_ident}"), fn_ident.span());
            let ret = &f.sig.output;
            let attrs = &f.attrs;
            let vis = &f.vis;

            let mut ptr_args = Vec::new();
            let mut wrapper_params = Vec::new();
            let mut call_args = Vec::new();

            for (i, arg) in f.sig.inputs.iter().enumerate() {
                if let syn::FnArg::Typed(pat_type) = arg {
                    let ty = &pat_type.ty;

                    let arg_name = if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                        pat_ident.ident.clone()
                    } else {
                        syn::Ident::new(&format!("a{i}"), proc_macro2::Span::call_site())
                    };

                    ptr_args.push(quote! { #ty });
                    wrapper_params.push(quote! { #arg_name: #ty });
                    call_args.push(quote! { #arg_name });
                }
            }

            out_items.push(quote! {
                #[unsafe(no_mangle)]
                static mut #pfn_ident: Option<unsafe extern "C" fn(#(#ptr_args),*) #ret> = None;
            });

            let expect_msg = format!("{fn_ident} not initialized by mpv");
            wrappers.push(quote! {
                #(#attrs)*
                #[inline]
                #vis unsafe fn #fn_ident(#(#wrapper_params),*) #ret {
                    unsafe { #pfn_ident.expect(#expect_msg)(#(#call_args),*) }
                }
            });
        }

        if !leftover.is_empty() {
            let attrs = &fm.attrs;
            let unsafety = &fm.unsafety;
            let abi = &fm.abi;
            out_items.push(quote! {
                #(#attrs)*
                #unsafety #abi {
                    #(#leftover)*
                }
            });
        }
    }

    out_items.extend(wrappers);

    let combined: TokenStream = out_items.into_iter().collect();
    let formatted = prettyplease::unparse(&syn::parse2::<syn::File>(combined).expect("reparse output"));
    fs::write(path, formatted).expect("write bindings");
}
