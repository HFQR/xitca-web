//! this is an example macro for xitca-postgres
//! it doesn't have any usability beyond as tutorial material.

use quote::quote;
use syn::{
    parse::{Parse, ParseStream},
    token::Comma,
    Expr, ExprReference, Lit, LitStr,
};

#[proc_macro]
pub fn sql(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let Query { sql, exprs, types } = syn::parse_macro_input!(input as Query);
    quote! {
        ::xitca_postgres::statement::Statement::unnamed(#sql, &[#(#types),*])
            .bind_dyn(&[#(#exprs),*])
    }
    .into()
}

struct Query {
    sql: LitStr,
    exprs: Vec<ExprReference>,
    types: Vec<proc_macro2::TokenStream>,
}

impl Parse for Query {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let sql = input.parse()?;
        let mut exprs = Vec::new();
        let mut types = Vec::new();

        while input.parse::<Comma>().is_ok() {
            let expr = input.parse::<ExprReference>()?;
            let Expr::Lit(lit) = expr.expr.as_ref() else {
                unimplemented!("example macro can only handle literal references")
            };
            let ty = match lit.lit {
                Lit::Str(_) => quote! { ::xitca_postgres::types::Type::TEXT },
                Lit::Int(_) => quote! { ::xitca_postgres::types::Type::INT4 },
                _ => unimplemented!("example macro only supports string and i32 as query parameters"),
            };
            types.push(ty);
            exprs.push(expr);
        }

        Ok(Self { sql, exprs, types })
    }
}
