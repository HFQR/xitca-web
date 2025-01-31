mod error;
mod route;
mod service;
mod state;

use proc_macro::TokenStream;
use syn::{Error, ImplItem, ImplItemFn, spanned::Spanned};

#[proc_macro_derive(State, attributes(borrow))]
pub fn state_impl(item: TokenStream) -> TokenStream {
    let item = syn::parse_macro_input!(item);
    state::state(item).unwrap_or_else(|e| e.to_compile_error().into())
}

/// attribute macro for `xitca-web` application.
///
/// # Pattern
/// ```plain
/// #[route("path", method = <method>[, attributes])]
/// ```
///
/// # Attributes
/// - `"path"`: string literal represent path register to http router.
///   `"/foo"` for example.  
/// - `method = <method>`: function path of http method register to http router.
///   `method = get` for example.
/// - `enclosed = <type>`: typed middleware applied to route.
/// - `enclosed_fn = <async function>`: async function as middleware applied to route
/// ```
#[proc_macro_attribute]
pub fn route(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = syn::parse_macro_input!(attr);
    let item = syn::parse_macro_input!(item);
    route::route(attr, item).unwrap_or_else(|e| e.to_compile_error().into())
}

#[proc_macro_attribute]
pub fn error_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = syn::parse_macro_input!(item);
    error::error(attr, item).unwrap_or_else(|e| e.to_compile_error().into())
}

#[proc_macro_attribute]
pub fn middleware_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = syn::parse_macro_input!(item);
    service::middleware(attr, item).unwrap_or_else(|e| e.to_compile_error().into())
}

#[proc_macro_attribute]
pub fn service_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = syn::parse_macro_input!(item);
    service::service(attr, item).unwrap_or_else(|e| e.to_compile_error().into())
}

fn find_async_method<'a>(items: &'a [ImplItem], ident_str: &str) -> Option<Result<&'a ImplItemFn, Error>> {
    for item in items.iter() {
        if let ImplItem::Fn(func) = item {
            if func.sig.ident.to_string().as_str() == ident_str {
                if func.sig.asyncness.is_none() {
                    return Some(Err(Error::new(
                        func.span(),
                        format!("{ident_str} method must be async fn"),
                    )));
                }

                return Some(Ok(func));
            }
        }
    }

    None
}
