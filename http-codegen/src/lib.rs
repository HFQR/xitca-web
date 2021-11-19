use proc_macro::TokenStream;
use quote::quote;
use syn::{FnArg, GenericArgument, ImplItem, ImplItemMethod, PathArguments, ReturnType, Type};

#[proc_macro_attribute]
pub fn service_impl(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(item as syn::ItemImpl);

    // Collect type path from impl.
    let service_ty = match input.self_ty.as_ref() {
        Type::Path(path) => path,
        _ => panic!("service_impl macro must be used on a TypePath"),
    };

    // collect generics.
    let generics = &input.generics;

    // find methods from impl.
    let new_service_impl =
        find_async_method(&input.items, "new_service").expect("new_service method can not be located");

    // collect ServiceFactory, Config and InitError type from new_service_impl
    let mut inputs = new_service_impl.sig.inputs.iter();
    let factory_ty = match inputs.next().unwrap() {
        FnArg::Receiver(_) => panic!("new_service method does not accept Self as receiver"),
        FnArg::Typed(ty) => match ty.ty.as_ref() {
            Type::Reference(ty_ref) if ty_ref.mutability.is_none() => &ty_ref.elem,
            _ => panic!("new_service must receive ServiceFactory type as immutable reference"),
        },
    };
    let config_ty = match inputs.next().unwrap() {
        FnArg::Receiver(_) => panic!("new_service method must not receive Self as receiver"),
        FnArg::Typed(ty) => &ty.ty,
    };
    let (_, init_err_ty) = extract_res_ty(&new_service_impl.sig.output);

    // make sure async fn ready is there and move on.
    // TODO: Check the first fn arg and make sure it's a Receiver of &Self.
    let _ = find_async_method(&input.items, "ready").expect("ready method can not be located");

    // collect Request, Response and Error type.
    let call_impl = find_async_method(&input.items, "call").expect("call method can not be located");

    let mut inputs = call_impl.sig.inputs.iter();
    // ignore receiver and move on.
    // TODO: Check the first fn arg and make sure it's a Receiver of &Self.
    let _ = inputs.next().unwrap();

    let req_ty = match inputs.next().unwrap() {
        FnArg::Receiver(_) => panic!("new_service method does not accept Self as receiver"),
        FnArg::Typed(ty) => &ty.ty,
    };

    let (res_ty, err_ty) = extract_res_ty(&call_impl.sig.output);

    let result = quote! {
        impl ::xitca_service::ServiceFactory<#req_ty> for #factory_ty {
            type Response = #res_ty;
            type Error = #err_ty;
            type Config = #config_ty;
            type Service = #service_ty;
            type InitError = #init_err_ty;
            type Future = impl ::core::future::Future<Output = Result<Self::Service, Self::InitError>>;

            fn new_service(&self, cfg: Self::Config) -> Self::Future {
                let this = self.clone();
                async move {
                    #service_ty::new_service(&this, cfg).await
                }
            }
        }

        impl<#generics> ::xitca_service::Service<#req_ty> for #service_ty {
            type Response = #res_ty;
            type Error = #err_ty;
            type Ready<'f> where Self: 'f = impl ::core::future::Future<Output = Result<(), Self::Error>>;
            type Future<'f> where Self: 'f = impl ::core::future::Future<Output = Result<Self::Response, Self::Error>>;

            #[inline]
            fn ready(&self) -> Self::Ready<'_> {
                async move {
                    #service_ty::ready(self).await
                }
            }

            #[inline]
            fn call(&self, req: #req_ty) -> Self::Future<'_> {
                async move {
                    #service_ty::call(self, req).await
                }
            }
        }

        #input
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
