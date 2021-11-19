use proc_macro::TokenStream;
use quote::{quote};
use syn::{Ident, ImplItem, ImplItemMethod, Type};


#[proc_macro_attribute]
pub fn service_impl(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(item as syn::ItemImpl);

    // Collect type path from impl.
    let ty = match input.self_ty.as_ref() {
        Type::Path(path) => path,
        _ => panic!("service_impl macro must be used on a TypePath")
    };

    // collect generics.
    let generics = &input.generics;

    // find methods from impl.
    let new_service_impl = find_async_method(&input.items, "new_service").expect("new_service method can not be located");

    // collect ServiceFactory, Config and InitError type from new_service_impl
    
    // println!("{:#?}", new_service_impl.sig);
    

    let ready_impl = find_async_method(&input.items, "ready").expect("ready method can not be located");
    let call_impl = find_async_method(&input.items, "call").expect("call method can not be located");

    let result = quote! {
        impl ::xitca_service::ServiceFactory<()> for TestFactory {
            type Response = ();
            type Error = ();
            type Config = ();
            type Service = #ty;
            type InitError = ();
            type Future = impl ::core::future::Future<Output = Result<Self::Service, Self::InitError>>;

            fn new_service(&self, cfg: Self::Config) -> Self::Future {
                let this = self.clone();
                async move {
                    #ty::new_service(&this, cfg).await
                }
            }
        }

        impl<#generics> ::xitca_service::Service<()> for #ty {
            type Response = ();
            type Error = ();
            type Ready<'f> where Self: 'f = impl ::core::future::Future<Output = Result<(), Self::Error>>;
            type Future<'f> where Self: 'f = impl ::core::future::Future<Output = Result<Self::Response, Self::Error>>;

            #[inline]
            fn ready(&self) -> Self::Ready<'_> {
                async move {
                    #ty::ready(self).await
                }
            }

            #[inline]
            fn call(&self, req: ()) -> Self::Future<'_> {
                async move {
                    #ty::call(self, req).await
                }
            }
        }

        #input
    };

    result.into()
}


fn find_async_method<'a>(items: &'a [ImplItem], ident_str: &'a str) -> Option<&'a ImplItemMethod> {
    items.iter()
        .find_map(|item| match item {
        ImplItem::Method(method) 
        if method.sig.ident.to_string().as_str() == ident_str 
         =>{
             assert!(method.sig.asyncness.is_some(), "{} method must be async fn", ident_str);
            Some(method)
         },
        _ => None,
    })
}