use proc_macro::TokenStream;
use quote::quote;
use syn::{
    Error, Expr, FnArg, GenericArgument, ItemFn, PathArguments, Type,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    spanned::Spanned,
};

pub(crate) fn route(attr: Args, input: ItemFn) -> Result<TokenStream, Error> {
    let mut attrs = attr.vars.into_iter();
    const MSG: &str = "not enough attributes. try add #[route(<path>, method = <method>)]";

    let path = attrs.next().ok_or_else(|| Error::new(input.span(), MSG))?;

    // TODO: add support for multiple method = x.
    let method = attrs.next().ok_or_else(|| Error::new(input.sig.ident.span(), MSG))?;
    let Expr::Assign(method) = method else {
        return Err(Error::new(method.span(), "expect 'method = <method>'"));
    };
    let method = &*method.right;

    let mut middlewares = quote! {};

    for attr in attrs {
        let Expr::Assign(pair) = attr else {
            return Err(Error::new(input.span(), "expect '<name> = <value>' expression"));
        };

        let Expr::Path(ref name) = *pair.left else {
            return Err(Error::new(pair.left.span(), "expect <name> to be path expression"));
        };

        let name = name
            .path
            .get_ident()
            .ok_or_else(|| Error::new(name.span(), "expect enclosed or enclosed_fn path"))?;
        match name.to_string().as_str() {
            "enclosed_fn" => {
                let Expr::Path(ref value) = *pair.right else {
                    return Err(Error::new(pair.right.span(), "expect <value> to be path expression"));
                };

                let value = value
                    .path
                    .get_ident()
                    .ok_or_else(|| Error::new(value.span(), "expect function name"))?;
                middlewares = quote! {
                    #middlewares.enclosed_fn(#value)
                };
            }
            "enclosed" => match *pair.right {
                Expr::Call(ref call) => {
                    middlewares = quote! {
                        #middlewares.enclosed(#call)
                    };
                }
                Expr::Path(ref path) => {
                    let value = path
                        .path
                        .get_ident()
                        .ok_or_else(|| Error::new(path.span(), "expect middleware type path"))?;

                    middlewares = quote! {
                        #middlewares.enclosed(#value)
                    };
                }
                _ => return Err(Error::new(pair.right.span(), "expect type path or function")),
            },
            _ => {}
        }
    }

    let is_async = input.sig.asyncness.is_some();
    let ident = &input.sig.ident;
    let vis = &input.vis;

    // handler argument may contain full or partial application state.
    // and the branching needed to handle differently.
    enum State<'a> {
        Partial(&'a Type),
        Full(&'a Type),
        Default,
    }

    let mut state = Vec::new();

    for arg in input.sig.inputs.iter() {
        if let FnArg::Typed(ty) = arg {
            let ty = match *ty.ty {
                Type::Path(ref ty) => ty,
                Type::Reference(ref ty) => match *ty.elem {
                    Type::Path(ref ty) => ty,
                    _ => continue,
                },
                _ => continue,
            };

            if let Some(path) = ty.path.segments.last() {
                let ident = path.ident.to_string();

                match ident.as_str() {
                    "StateOwn" | "StateRef" => {
                        let PathArguments::AngleBracketed(ref arg) = path.arguments else {
                            return Err(Error::new(path.span(), format!("expect {ident}<_>")));
                        };
                        match arg.args.last() {
                            Some(GenericArgument::Type(ty)) => {
                                state.push(State::Partial(ty));
                            }
                            _ => return Err(Error::new(ty.span(), "expect state type.")),
                        }
                    }
                    "WebContext" => {
                        let PathArguments::AngleBracketed(ref arg) = path.arguments else {
                            return Err(Error::new(path.span(), format!("expect &{ident}<'_, _>")));
                        };

                        let mut args = arg.args.iter();

                        match args.next() {
                            Some(GenericArgument::Lifetime(..)) => {}
                            _ => return Err(Error::new(ty.span(), "expect lifetime param '_")),
                        }

                        match args.next() {
                            Some(GenericArgument::Type(ty)) => {
                                state.push(State::Full(ty));
                            }
                            None => {
                                state.push(State::Default);
                            }
                            _ => return Err(Error::new(ty.span(), "expect state type.")),
                        };

                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    let mut generic_arg = quote! { <C> };
    let mut where_clause = quote! { 'static };
    // sync middleware needs to clone the state and send it to another thread.
    if !is_async {
        where_clause = quote! { Send + Sync + Clone + #where_clause };
    }
    let mut state_ident = quote! { C };

    where_clause = quote! { where #state_ident: #where_clause };

    if !state.is_empty() {
        for state in state.into_iter() {
            match state {
                State::Partial(ty) => {
                    where_clause = quote! {
                        #where_clause + ::xitca_web::handler::state::BorrowState<#ty>
                    };
                }
                State::Full(ty) => {
                    generic_arg = quote! {};
                    where_clause = quote! {};
                    state_ident = quote! { #ty };
                    break;
                }
                State::Default => {
                    generic_arg = quote! {};
                    where_clause = quote! {};
                    state_ident = quote! {};
                }
            }
        }
    }

    let handler = if is_async {
        quote! { ::xitca_web::handler::handler_service }
    } else {
        quote! { ::xitca_web::handler::handler_sync_service }
    };

    Ok(quote! {
        #[allow(non_camel_case_types)]
        #vis struct #ident;

        impl #generic_arg ::xitca_web::codegen::__private::TypedRoute<#state_ident> for #ident
        #where_clause
        {
            type Route = ::xitca_web::codegen::__private::RouteObject<
                (),
                Box<dyn for<'r> ::xitca_web::service::object::ServiceObject<
                    ::xitca_web::WebContext<'r, #state_ident>,
                    Response = ::xitca_web::http::WebResponse,
                    Error = ::xitca_web::error::RouterError<::xitca_web::error::Error>
                >>,
                ::core::convert::Infallible
            >;

            fn path() -> &'static str {
                #path
            }

            fn route() -> Self::Route {
                #input

                use xitca_web::codegen::__private::IntoObject;
                use xitca_web::WebContext;
                use xitca_web::route::#method;
                use xitca_web::service::ServiceExt;

                WebContext::<'_, #state_ident>::into_object(#method(#handler(#ident)#middlewares))
            }
        }
    }
    .into())
}

pub struct Args {
    vars: Vec<Expr>,
}

impl Parse for Args {
    fn parse(input: ParseStream) -> Result<Self, Error> {
        let vars = Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated(input)?;

        Ok(Args {
            vars: vars.into_iter().collect(),
        })
    }
}
