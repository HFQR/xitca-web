use proc_macro::TokenStream;
use quote::quote;
use syn::{Error, ItemImpl, Type, spanned::Spanned};

pub(crate) fn error(_: TokenStream, item: ItemImpl) -> Result<TokenStream, Error> {
    let Type::Path(ref err_ty) = *item.self_ty else {
        return Err(Error::new(item.self_ty.span(), "expect Struct or Enum"));
    };

    let call_impl = crate::find_async_method(&item.items, "call")
        .ok_or_else(|| Error::new(err_ty.span(), "expect 'async fn call' method"))??;

    let call_stmts = &call_impl.block.stmts;

    Ok(quote! {
        impl<'r> ::xitca_web::service::Service<WebContext<'r, ::xitca_web::error::Request<'r>>> for #err_ty {
            type Response = WebResponse;
            type Error = ::core::convert::Infallible;

            async fn call(&self, ctx: WebContext<'r, ::xitca_web::error::Request<'r>>) -> Result<Self::Response, Self::Error> {
                Ok({#(#call_stmts)*})
            }
        }

        impl From<#err_ty> for ::xitca_web::error::Error {
            fn from(e: #err_ty) -> Self {
                Self::from_service(e)
            }
        }
    }
    .into())
}
