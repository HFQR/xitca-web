use proc_macro::TokenStream;
use quote::quote;
use syn::{
    __private::{Span, TokenStream2},
    Error, FnArg, GenericArgument, GenericParam, Ident, ImplItemFn, ItemImpl, Pat, PatIdent, PathArguments, ReturnType,
    Stmt, Type, TypePath, WhereClause,
    punctuated::Punctuated,
    spanned::Spanned,
    token::Comma,
};

use crate::find_async_method;

pub(crate) fn middleware(_: TokenStream, input: ItemImpl) -> Result<TokenStream, Error> {
    let builder_impl = BuilderImpl::try_from_item(&input)?;
    let call_stream = CallImpl::try_from_item(&input)?.into_token_stream(&builder_impl);

    let BuilderImpl {
        service_ty,
        generics,
        where_clause,
        impl_fn,
        builder_ident,
        builder_ty,
        builder_stmts,
        arg_ident,
        arg_ty,
    } = builder_impl;

    let new_service_generic_err_ty = impl_fn
        .sig
        .generics
        .params
        .first()
        .ok_or_else(|| Error::new(impl_fn.sig.generics.span(), "expect generic E type"))?;

    let new_service_rt_err_ty = find_new_service_rt_err_ty(impl_fn)?;

    let base = quote! {
        impl<#generics, #new_service_generic_err_ty> ::xitca_service::Service<#arg_ty> for #builder_ty
        #where_clause
        {
            type Response = #service_ty;
            type Error = #new_service_rt_err_ty;

            async fn call(&self, #arg_ident: #arg_ty) -> Result<Self::Response, Self::Error> {
                let #builder_ident = &self;
                #(#builder_stmts)*
            }
        }

        #call_stream
    };

    let ready_impl = ReadyImpl::try_from_item(&input)?;
    Ok(try_extend_ready_impl(
        base,
        ready_impl,
        service_ty,
        generics,
        where_clause,
    ))
}

pub(crate) fn service(_: TokenStream, input: ItemImpl) -> Result<TokenStream, Error> {
    let builder_impl = BuilderImpl::try_from_item(&input)?;
    let call_impl = CallImpl::try_from_item(&input)?;

    let err_ty = call_impl.err_ty;

    let call_stream = call_impl.into_token_stream(&builder_impl);

    let BuilderImpl {
        service_ty,
        generics,
        where_clause,
        builder_ident,
        builder_ty,
        builder_stmts,
        arg_ident,
        arg_ty,
        ..
    } = builder_impl;

    let base = quote! {
        impl<#generics> ::xitca_service::Service<#arg_ty> for #builder_ty
        #where_clause
        {
            type Response = #service_ty;
            type Error = #err_ty;

            async fn call(&self, #arg_ident: #arg_ty) -> Result<Self::Response, Self::Error> {
                let #builder_ident = &self;
                #(#builder_stmts)*
            }
        }

        #call_stream
    };

    let ready_impl = ReadyImpl::try_from_item(&input)?;
    Ok(try_extend_ready_impl(
        base,
        ready_impl,
        service_ty,
        generics,
        where_clause,
    ))
}

fn try_extend_ready_impl(
    base: TokenStream2,
    ready_impl: Option<ReadyImpl<'_>>,
    service_ty: &TypePath,
    generics: &Punctuated<GenericParam, Comma>,
    where_clause: Option<&WhereClause>,
) -> TokenStream {
    let ready = ready_impl.map(
        |ReadyImpl {
             ready_stmts,
             ready_ret_ty,
         }| {
            quote! {
                impl<#generics> ::xitca_service::ready::ReadyService for #service_ty
                #where_clause
                {
                    type Ready = #ready_ret_ty;

                    #[inline]
                    async fn ready(&self) -> Self::Ready {
                        #(#ready_stmts)*
                    }
                }
            }
        },
    );

    quote! {
        #base
        #ready
    }
    .into()
}

struct BuilderImpl<'a> {
    service_ty: &'a TypePath,
    generics: &'a Punctuated<GenericParam, Comma>,
    where_clause: Option<&'a WhereClause>,
    impl_fn: &'a ImplItemFn,
    builder_ident: PatIdent,
    builder_ty: &'a Type,
    builder_stmts: &'a [Stmt],
    arg_ident: PatIdent,
    arg_ty: &'a Type,
}

impl<'a> BuilderImpl<'a> {
    fn try_from_item(input: &'a ItemImpl) -> Result<BuilderImpl<'a>, Error> {
        // Collect type path from impl.
        let Type::Path(ref service_ty) = *input.self_ty else {
            return Err(Error::new(input.self_ty.span(), "expect Struct or Enum"));
        };

        // collect generics.
        let generics = &input.generics.params;
        let where_clause = input.generics.where_clause.as_ref();

        // find methods from impl.
        let impl_fn = find_async_method(&input.items, "new_service")
            .ok_or_else(|| Error::new(input.span(), "can't find 'async fn new_service'"))??;

        // collect builder info
        let builder_stmts = impl_fn.block.stmts.as_slice();

        let mut inputs = impl_fn.sig.inputs.iter();

        let (builder_ident, builder_ty) = match inputs.next().ok_or_else(|| {
            Error::new(
                impl_fn.sig.inputs.span(),
                "expect Builder type as the first 'new_service' function argument",
            )
        })? {
            FnArg::Receiver(recv) => {
                return Err(Error::new(
                    recv.span(),
                    "new_service method does not accept Self as receiver",
                ));
            }
            FnArg::Typed(ty) => match (ty.pat.as_ref(), ty.ty.as_ref()) {
                (Pat::Wild(_), Type::Reference(ty_ref)) if ty_ref.mutability.is_none() => {
                    (default_pat_ident("_factory"), &ty_ref.elem)
                }
                (Pat::Ident(ident), Type::Reference(ty_ref)) if ty_ref.mutability.is_none() => {
                    (ident.to_owned(), &ty_ref.elem)
                }
                _ => {
                    return Err(Error::new(
                        ty.span(),
                        "new_service must receive ServiceFactory type as immutable reference",
                    ));
                }
            },
        };

        let (call_ident, call_ty) = match inputs.next().ok_or_else(|| {
            Error::new(
                impl_fn.sig.inputs.span(),
                "expect generic Arg type as the second 'new_service' function argument",
            )
        })? {
            FnArg::Receiver(recv) => {
                return Err(Error::new(
                    recv.span(),
                    "new_service method does not accept Self as receiver",
                ));
            }
            FnArg::Typed(ty) => match ty.pat.as_ref() {
                Pat::Wild(_) => (default_pat_ident("_service"), &*ty.ty),
                Pat::Ident(ident) => (ident.to_owned(), &*ty.ty),
                _ => {
                    return Err(Error::new(
                        ty.span(),
                        "new_service method must use 'arg: Arg' as second function argument",
                    ));
                }
            },
        };

        Ok(Self {
            service_ty,
            generics,
            where_clause,
            impl_fn,
            builder_ident,
            builder_ty,
            builder_stmts,
            arg_ident: call_ident,
            arg_ty: call_ty,
        })
    }
}

struct CallImpl<'a> {
    req_ident: PatIdent,
    req_ty: &'a Type,
    res_ty: &'a Type,
    err_ty: &'a Type,
    call_stmts: &'a [Stmt],
}

impl<'a> CallImpl<'a> {
    fn try_from_item(item: &'a ItemImpl) -> Result<Self, Error> {
        // collect Request, Response and Error type.
        let call_impl = find_async_method(&item.items, "call")
            .ok_or_else(|| Error::new(item.span(), "can't find 'async fn call'"))??;

        let mut inputs = call_impl.sig.inputs.iter();
        // ignore receiver and move on.
        // TODO: Check the first fn arg and make sure it's a Receiver of &Self.
        let _ = inputs.next().ok_or_else(|| {
            Error::new(
                call_impl.sig.inputs.span(),
                "expect &Self as the first 'call' function argument",
            )
        })?;

        let (req_ident, req_ty) = match inputs.next().ok_or_else(|| {
            Error::new(
                call_impl.sig.inputs.span(),
                "expect generic Req type as the second 'call' function argument",
            )
        })? {
            FnArg::Receiver(recv) => {
                return Err(Error::new(recv.span(), "call method does not accept Self as receiver"));
            }
            FnArg::Typed(ty) => match ty.pat.as_ref() {
                Pat::Wild(_) => (default_pat_ident("_req"), &*ty.ty),
                Pat::Ident(ident) => (ident.to_owned(), &*ty.ty),
                _ => {
                    return Err(Error::new(
                        ty.span(),
                        "call method must use 'req: Req' as second function argument",
                    ));
                }
            },
        };
        let (res_ty, err_ty) = res_ty(&call_impl.sig.output)?;
        let call_stmts = &call_impl.block.stmts;

        Ok(Self {
            req_ident,
            req_ty,
            res_ty,
            err_ty,
            call_stmts,
        })
    }

    fn into_token_stream(self, builder_impl: &BuilderImpl<'_>) -> TokenStream2 {
        let Self {
            req_ident,
            req_ty,
            res_ty,
            err_ty,
            call_stmts,
        } = self;

        let service_ty = builder_impl.service_ty;
        let generics = builder_impl.generics;
        let where_clause = builder_impl.where_clause;

        quote! {
            impl<#generics> ::xitca_service::Service<#req_ty> for #service_ty
            #where_clause
            {
                type Response = #res_ty;
                type Error = #err_ty;

                #[inline]
                async fn call(&self, #req_ident: #req_ty) -> Result<Self::Response, Self::Error> {
                    #(#call_stmts)*
                }
            }
        }
    }
}

struct ReadyImpl<'a> {
    ready_stmts: &'a [Stmt],
    ready_ret_ty: &'a Type,
}

impl<'a> ReadyImpl<'a> {
    fn try_from_item(item: &'a ItemImpl) -> Result<Option<Self>, Error> {
        let items = &item.items;
        let ready = find_async_method(items, "ready").transpose()?;
        match ready {
            Some(ready_impl) => {
                let ready_ret_ty = ready_ret_ty(&ready_impl.sig.output)?;
                let ready_stmts = &ready_impl.block.stmts;
                Ok(Some(Self {
                    ready_stmts,
                    ready_ret_ty,
                }))
            }
            None => Ok(None),
        }
    }
}

// Extract Result<T, E> types from a return type of function.
fn res_ty(ret: &ReturnType) -> Result<(&Type, &Type), Error> {
    if let ReturnType::Type(_, ty) = ret {
        if let Type::Path(path) = ty.as_ref() {
            if let Some(sig) = path.path.segments.first() {
                if sig.ident == "Result" {
                    if let PathArguments::AngleBracketed(ref arg) = sig.arguments {
                        if let (Some(GenericArgument::Type(ok_ty)), Some(GenericArgument::Type(err_ty))) =
                            (arg.args.first(), arg.args.last())
                        {
                            return Ok((ok_ty, err_ty));
                        }
                    }
                }
            }
        }
    }

    Err(Error::new(ret.span(), "expect Result<Self, <Error>> as return type"))
}

// Extract types from a return type of function.
fn ready_ret_ty(ret: &ReturnType) -> Result<&Type, Error> {
    match ret {
        ReturnType::Type(_, ty) => Ok(ty),
        _ => Err(Error::new(ret.span(), "expect ReadyService::Ready as return type")),
    }
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

fn find_new_service_rt_err_ty(func: &ImplItemFn) -> Result<&Type, Error> {
    if let ReturnType::Type(_, ref new_service_rt_ty) = func.sig.output {
        if let Type::Path(ref path) = **new_service_rt_ty {
            if let Some(path) = path.path.segments.first() {
                if path.ident == "Result" {
                    if let PathArguments::AngleBracketed(ref bracket) = path.arguments {
                        if let Some(GenericArgument::Type(ty)) = bracket.args.last() {
                            return Ok(ty);
                        }
                    };
                }
            }
        };
    }

    Err(Error::new(func.sig.output.span(), "expect Result<_, _> as return type"))
}
