use proc_macro::TokenStream;
use quote::{__private::Span, quote};
use syn::{
    Data, FnArg, GenericArgument, Ident, ImplItem, ImplItemMethod, Pat, PatIdent, PathArguments, ReturnType, Stmt, Type,
};

#[proc_macro_derive(State, attributes(borrow))]
pub fn state_impl(item: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(item as syn::DeriveInput);

    let ty_ident = &input.ident;
    let _generics = &input.generics;
    let ty = match input.data {
        Data::Struct(ref ty) => ty,
        _ => todo!(),
    };

    let fields = ty
        .fields
        .iter()
        .enumerate()
        .filter(|(_, field)| {
            field.attrs.iter().any(|attr| {
                attr.path
                    .segments
                    .first()
                    .filter(|seg| seg.ident.to_string().as_str() == "borrow")
                    .is_some()
            })
        })
        .map(|(_, field)| {
            let ident = field.ident.as_ref().unwrap();
            let ty = &field.ty;

            quote! {
                impl ::core::borrow::Borrow<#ty> for #ty_ident {
                    fn borrow(&self) -> &#ty {
                        &self.#ident
                    }
                }
            }
        });

    quote! { #(#fields)* }.into()
}

#[proc_macro_attribute]
pub fn service_impl(_attr: TokenStream, item: TokenStream) -> TokenStream {
    middleware_impl(_attr, item)
}

#[proc_macro_attribute]
pub fn middleware_impl(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(item as syn::ItemImpl);

    // Collect type path from impl.
    let service_ty = match input.self_ty.as_ref() {
        Type::Path(path) => path,
        _ => panic!("impl macro must be used on a TypePath"),
    };

    // collect generics.
    let generic_ty = &input.generics.params;
    let where_clause = &input.generics.where_clause;

    // find methods from impl.
    let new_service_impl =
        find_async_method(&input.items, "new_service").expect("new_service method can not be located");

    // collect ServiceFactory type
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
    let (arg_ident, arg_ty) = match inputs.next().unwrap() {
        FnArg::Receiver(_) => panic!("new_service method must not receive Self as receiver"),
        FnArg::Typed(ty) => match ty.pat.as_ref() {
            Pat::Wild(_) => (default_pat_ident("_service"), &*ty.ty),
            Pat::Ident(ident) => (ident.to_owned(), &*ty.ty),
            _ => panic!("new_service method must use <arg: Arg> as second function argument"),
        },
    };

    let factory_stmts = &new_service_impl.block.stmts;

    let ready_impl = ReadyImpl::try_from_items(&input.items);

    let CallImpl {
        req_ident,
        req_ty,
        res_ty,
        err_ty,
        call_stmts,
    } = CallImpl::from_items(&input.items);

    let base = quote! {
        impl<#generic_ty> ::xitca_service::Service<#arg_ty> for #factory_ty
        #where_clause
        {
            type Response = #service_ty;
            type Error = #err_ty;
            type Future<'f> = impl ::core::future::Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, #arg_ty: 'f;

            fn call<'s, 'f>(&'s self, #arg_ident: #arg_ty) -> Self::Future<'f> where 's: 'f, #arg_ty: 'f {
                async move {
                    let #factory_ident = &self;
                    #(#factory_stmts)*
                }
            }
        }

        impl<#generic_ty> ::xitca_service::Service<#req_ty> for #service_ty
        #where_clause
        {
            type Response = #res_ty;
            type Error = #err_ty;
            type Future<'f> = impl ::core::future::Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, #req_ty: 'f;

            #[inline]
            fn call<'s, 'f>(&'s self, #req_ident: #req_ty) -> Self::Future<'f> where 's: 'f, #req_ty: 'f {
                async move {
                    #(#call_stmts)*
                }
            }
        }
    };

    match ready_impl {
        Some(ReadyImpl {
            ready_stmts,
            ready_ret_ty: ready_res_ty,
        }) => quote! {
            #base

            impl<#generic_ty> ::xitca_service::ready::ReadyService for #service_ty
            #where_clause
            {
                type Ready = #ready_res_ty;
                type ReadyFuture<'f> = impl ::core::future::Future<Output = Self::Ready> + 'f where Self: 'f;
                #[inline]
                fn ready(&self) -> Self::ReadyFuture<'_> {
                    async move {
                        #(#ready_stmts)*
                    }
                }
            }
        }
        .into(),
        None => base.into(),
    }
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
    ready_ret_ty: &'a Type,
}

impl<'a> ReadyImpl<'a> {
    fn try_from_items(items: &'a [ImplItem]) -> Option<Self> {
        find_async_method(items, "ready").map(|ready_impl| {
            let ready_ret_ty = ready_ret_ty(&ready_impl.sig.output);
            let ready_stmts = &ready_impl.block.stmts;

            Self {
                ready_stmts,
                ready_ret_ty,
            }
        })
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

    panic!("build method must output Result<Self, <Error>>")
}

// Extract types from a return type of function.
fn ready_ret_ty(ret: &ReturnType) -> &Type {
    if let ReturnType::Type(_, ty) = ret {
        return ty;
    }

    panic!("ready method must output ReadyService::Ready type")
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
