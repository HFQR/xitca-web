use proc_macro::TokenStream;
use quote::{__private::Span, quote};
use syn::{
    FnArg, GenericArgument, Ident, ImplItem, ImplItemMethod, Pat, PatIdent, PathArguments, ReturnType, Stmt, Type,
};

#[proc_macro_attribute]
pub fn service_impl(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(item as syn::ItemImpl);

    // Collect type path from impl.
    let service_ty = match input.self_ty.as_ref() {
        Type::Path(path) => path,
        _ => panic!("service_impl macro must be used on a TypePath"),
    };

    // collect generics.
    let generic_ty = &input.generics.params;
    let where_clause = &input.generics.where_clause;

    // find methods from impl.
    let new_service_impl =
        find_async_method(&input.items, "new_service").expect("new_service method can not be located");

    // collect ServiceFactory, Config and InitError type from new_service_impl
    let mut inputs = new_service_impl.sig.inputs.iter();

    let (factory_ident, factory_ty) = match inputs.next().unwrap() {
        FnArg::Receiver(_) => panic!("new_service method does not accept Self as receiver"),
        FnArg::Typed(ty) => match (ty.pat.as_ref(), ty.ty.as_ref()) {
            (Pat::Wild(_), Type::Reference(ty_ref)) if ty_ref.mutability.is_none() => {
                (default_pat_ident("_factory"), &ty_ref.elem)
            }
            (Pat::Ident(ident), Type::Reference(ty_ref)) if ty_ref.mutability.is_none() => {
                (ident.to_owned(), &ty_ref.elem)
            }
            _ => panic!("new_service must receive ServiceFactory type as immutable reference"),
        },
    };
    let (cfg_ident, cfg_ty) = match inputs.next().unwrap() {
        FnArg::Receiver(_) => panic!("new_service method must not receive Self as receiver"),
        FnArg::Typed(ty) => match ty.pat.as_ref() {
            Pat::Wild(_) => (default_pat_ident("_cfg"), &*ty.ty),
            Pat::Ident(ident) => (ident.to_owned(), &*ty.ty),
            _ => panic!("new_service method must use cfg: Config as second function argument"),
        },
    };
    let (_, init_err_ty) = extract_res_ty(&new_service_impl.sig.output);
    let factory_stmts = &new_service_impl.block.stmts;

    let ReadyImpl { ready_stmts } = ReadyImpl::from_items(&input.items);

    let CallImpl {
        req_ident,
        req_ty,
        res_ty,
        err_ty,
        call_stmts,
    } = CallImpl::from_items(&input.items);

    let result = quote! {
        impl ::xitca_service::ServiceFactory<#req_ty> for #factory_ty {
            type Response = #res_ty;
            type Error = #err_ty;
            type Config = #cfg_ty;
            type Service = #service_ty;
            type InitError = #init_err_ty;
            type Future = impl ::core::future::Future<Output = Result<Self::Service, Self::InitError>>;

            fn new_service(&self, cfg: Self::Config) -> Self::Future {
                let #factory_ident = self.clone();
                let #cfg_ident = cfg;
                async move {
                    #(#factory_stmts)*
                }
            }
        }

        impl<#generic_ty> ::xitca_service::Service<#req_ty> for #service_ty
        #where_clause
        {
            type Response = #res_ty;
            type Error = #err_ty;
            type Ready<'f> where Self: 'f = impl ::core::future::Future<Output = Result<(), Self::Error>>;
            type Future<'f> where Self: 'f = impl ::core::future::Future<Output = Result<Self::Response, Self::Error>>;

            #[inline]
            fn ready(&self) -> Self::Ready<'_> {
                async move {
                    #(#ready_stmts)*
                }
            }

            #[inline]
            fn call(&self, req: #req_ty) -> Self::Future<'_> {
                let #req_ident = req;
                async move {
                    #(#call_stmts)*
                }
            }
        }
    };

    result.into()
}

#[proc_macro_attribute]
pub fn middleware_impl(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(item as syn::ItemImpl);

    // Collect type path from impl.
    let middleware_ty = match input.self_ty.as_ref() {
        Type::Path(path) => path,
        _ => panic!("middleware_impl macro must be used on a TypePath"),
    };

    // collect generics.
    let generic_ty = &input.generics.params;
    let where_clause = &input.generics.where_clause;

    // find methods from impl.
    let new_transform_impl =
        find_async_method(&input.items, "new_transform").expect("new_transform method can not be located");

    // collect ServiceFactory, Config and InitError type from new_transform_impl
    let mut inputs = new_transform_impl.sig.inputs.iter();

    let (transform_ident, transform_ty) = match inputs.next().unwrap() {
        FnArg::Receiver(_) => panic!("new_transform method does not accept Self as receiver"),
        FnArg::Typed(ty) => match (ty.pat.as_ref(), ty.ty.as_ref()) {
            (Pat::Wild(_), Type::Reference(ty_ref)) if ty_ref.mutability.is_none() => {
                (default_pat_ident("_factory"), &ty_ref.elem)
            }
            (Pat::Ident(ident), Type::Reference(ty_ref)) if ty_ref.mutability.is_none() => {
                (ident.to_owned(), &ty_ref.elem)
            }
            _ => panic!("new_transform must receive ServiceFactory type as immutable reference"),
        },
    };
    let (service_ident, service_ty) = match inputs.next().unwrap() {
        FnArg::Receiver(_) => panic!("new_transform method must not receive Self as receiver"),
        FnArg::Typed(ty) => match ty.pat.as_ref() {
            Pat::Wild(_) => (default_pat_ident("_service"), &*ty.ty),
            Pat::Ident(ident) => (ident.to_owned(), &*ty.ty),
            _ => panic!("new_transform method must use cfg: Config as second function argument"),
        },
    };
    let (_, init_err_ty) = extract_res_ty(&new_transform_impl.sig.output);
    let tranform_stmts = &new_transform_impl.block.stmts;

    let ReadyImpl { ready_stmts } = ReadyImpl::from_items(&input.items);

    let CallImpl {
        req_ident,
        req_ty,
        res_ty,
        err_ty,
        call_stmts,
    } = CallImpl::from_items(&input.items);

    let result = quote! {
        impl<#generic_ty> ::xitca_service::Transform<#service_ty, #req_ty> for #transform_ty
        #where_clause
        {
            type Response = #res_ty;
            type Error = #err_ty;
            type Transform = #middleware_ty;
            type InitError = #init_err_ty;
            type Future = impl ::core::future::Future<Output = Result<Self::Transform, Self::InitError>>;

            fn new_transform(&self, service: #service_ty) -> Self::Future {
                let #transform_ident = self.clone();
                let #service_ident = service;
                async move {
                    #(#tranform_stmts)*
                }
            }
        }

        impl<#generic_ty> ::xitca_service::Service<#req_ty> for #middleware_ty
        #where_clause
        {
            type Response = #res_ty;
            type Error = #err_ty;
            type Ready<'f> where Self: 'f = impl ::core::future::Future<Output = Result<(), Self::Error>>;
            type Future<'f> where Self: 'f = impl ::core::future::Future<Output = Result<Self::Response, Self::Error>>;

            #[inline]
            fn ready(&self) -> Self::Ready<'_> {
                async move {
                    #(#ready_stmts)*
                }
            }

            #[inline]
            fn call(&self, req: #req_ty) -> Self::Future<'_> {
                let #req_ident = req;
                async move {
                    #(#call_stmts)*
                }
            }
        }
    };

    result.into()
}

fn find_async_method<'a>(items: &'a [ImplItem], ident_str: &'a str) -> Option<&'a ImplItemMethod> {
    items.iter().find_map(|item| match item {
        ImplItem::Method(method) if method.sig.ident.to_string().as_str() == ident_str => {
            assert!(method.sig.asyncness.is_some(), "{} method must be async fn", ident_str);
            Some(method)
        }
        _ => None,
    })
}

struct CallImpl<'a> {
    req_ident: PatIdent,
    req_ty: &'a Type,
    res_ty: &'a Type,
    err_ty: &'a Type,
    call_stmts: &'a [Stmt],
}

impl<'a> CallImpl<'a> {
    fn from_items(items: &'a [ImplItem]) -> Self {
        // collect Request, Response and Error type.
        let call_impl = find_async_method(items, "call").expect("call method can not be located");

        let mut inputs = call_impl.sig.inputs.iter();
        // ignore receiver and move on.
        // TODO: Check the first fn arg and make sure it's a Receiver of &Self.
        let _ = inputs.next().unwrap();

        let (req_ident, req_ty) = match inputs.next().unwrap() {
            FnArg::Receiver(_) => panic!("call method does not accept Self as second argument"),
            FnArg::Typed(ty) => match ty.pat.as_ref() {
                Pat::Wild(_) => (default_pat_ident("_req"), &*ty.ty),
                Pat::Ident(ident) => (ident.to_owned(), &*ty.ty),
                _ => panic!("call must use req: Request as second function argument"),
            },
        };
        let (res_ty, err_ty) = extract_res_ty(&call_impl.sig.output);
        let call_stmts = &call_impl.block.stmts;

        CallImpl {
            req_ident,
            req_ty,
            res_ty,
            err_ty,
            call_stmts,
        }
    }
}

struct ReadyImpl<'a> {
    ready_stmts: &'a [Stmt],
}

impl<'a> ReadyImpl<'a> {
    fn from_items(items: &'a [ImplItem]) -> Self {
        // make sure async fn ready is there and move on.
        // TODO: Check the first fn arg and make sure it's a Receiver of &Self.

        let ready_impl = find_async_method(items, "ready").expect("ready method can not be located");
        let ready_stmts = &ready_impl.block.stmts;

        Self { ready_stmts }
    }
}

// Extract Result<T, E> types from a return type of function.
fn extract_res_ty(ret: &ReturnType) -> (&Type, &Type) {
    if let ReturnType::Type(_, ty) = ret {
        if let Type::Path(path) = ty.as_ref() {
            let seg = path.path.segments.first().unwrap();
            if seg.ident.to_string().as_str() == "Result" {
                if let PathArguments::AngleBracketed(ref arg) = seg.arguments {
                    if let (Some(GenericArgument::Type(ok_ty)), Some(GenericArgument::Type(err_ty))) =
                        (arg.args.first(), arg.args.last())
                    {
                        return (ok_ty, err_ty);
                    }
                }
            }
        }
    }

    panic!("new_service method must output Result<Self, <InitError>>")
}

// generate a default PatIdent
fn default_pat_ident(ident: &str) -> PatIdent {
    PatIdent {
        attrs: Vec::with_capacity(0),
        by_ref: None,
        mutability: None,
        ident: Ident::new(ident, Span::call_site()),
        subpat: None,
    }
}
