use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Error};

pub(crate) fn state(input: DeriveInput) -> Result<TokenStream, Error> {
    let ty_ident = &input.ident;
    let _generics = &input.generics;

    let Data::Struct(ref ty) = input.data else {
        return Err(Error::new(ty_ident.span(), "expect Struct"));
    };

    let fields = ty
        .fields
        .iter()
        .enumerate()
        .filter(|(_, field)| {
            field.attrs.iter().any(|attr| {
                attr.path()
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

    Ok(quote! { #(#fields)* }.into())
}
