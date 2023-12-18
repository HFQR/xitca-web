use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    spanned::Spanned,
    Error, Expr, FnArg, GenericArgument, ItemFn, PathArguments, Type,
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

        let Expr::Path(ref value) = *pair.right else {
            return Err(Error::new(pair.left.span(), "expect <name> to be path expression"));
        };

        let name = name
            .path
            .get_ident()
            .ok_or_else(|| Error::new(name.span(), "expect enclosed or enclosed_fn path"))?;
        match name.to_string().as_str() {
            "enclosed_fn" => {
                let value = value
                    .path
                    .get_ident()
                    .ok_or_else(|| Error::new(value.span(), "expect function name"))?;
                middlewares = quote! {
                    #middlewares.enclosed_fn(#value)
                };
            }
            "enclosed" => return Err(Error::new(name.span(), "enclosed is not supported yet")),
            _ => {}
        }
    }

    let is_async = input.sig.asyncness.is_some();
    let ident = &input.sig.ident;
    let vis = &input.vis;

    let mut state = None;

    for arg in input.sig.inputs.iter() {
        if let FnArg::Typed(ty) = arg {
            if let Type::Path(ref ty) = *ty.ty {
                if let Some(path) = ty.path.segments.last() {
                    let ident = path.ident.to_string();

                    if ident == "StateOwn" || ident == "StateRef" {
                        let PathArguments::AngleBracketed(ref arg) = path.arguments else {
                            return Err(Error::new(path.span(), format!("expect {ident}<_>")));
                        };

                        match arg.args.last() {
                            Some(GenericArgument::Type(ref ty)) => {
                                state = Some(ty);
                            }
                            _ => return Err(Error::new(ty.span(), "expect state type.")),
                        }
                    }
                }
            }
        }
    }

    let state = state.map(|path| {
        quote! {
            ::core::borrow::Borrow<#path> +
        }
    });

    let handler = if is_async {
        quote! { ::xitca_web::handler::handler_service }
    } else {
        quote! { ::xitca_web::handler::handler_sync_service }
    };

    Ok(quote! {
        #[allow(non_camel_case_types)]
        #vis struct #ident;

        impl<C, B> ::xitca_web::codegen::__private::TypedRoute<(C, B)> for #ident
        where
            C: #state 'static,
            B: ::xitca_web::body::BodyStream + 'static
        {
            type Route = ::xitca_web::dev::service::object::BoxedSyncServiceObject<
                (),
                Box<dyn for<'r> ::xitca_web::dev::service::object::ServiceObject<
                    ::xitca_web::WebContext<'r, C, B>,
                    Response = ::xitca_web::http::WebResponse,
                    Error = ::xitca_web::error::RouterError<::xitca_web::error::Error<C>>
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
                use xitca_web::dev::service::ServiceExt;

                WebContext::<'_, C, B>::into_object(#method(#handler(#ident)#middlewares))
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
