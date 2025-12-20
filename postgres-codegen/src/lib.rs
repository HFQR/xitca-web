//! this is an example macro for xitca-postgres
//! it doesn't have any usability beyond as tutorial material.

use quote::quote;
use sqlparser::{dialect::PostgreSqlDialect, parser::Parser};
use syn::{
    parse::{Parse, ParseStream},
    spanned::Spanned,
    token::Comma,
    Expr, ExprReference, Lit, LitStr,
};

#[proc_macro]
pub fn sql(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let Query { sql, exprs, types } = syn::parse_macro_input!(input as Query);

    quote! {
        ::xitca_postgres::statement::Statement::named(#sql, &[#(#types),*])
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
        let sql = input.parse::<LitStr>()?;

        Parser::parse_sql(&PostgreSqlDialect {}, &sql.value())
            .map_err(|e| syn::Error::new(sql.span(), e.to_string()))?;

        let mut exprs = Vec::new();
        let mut types = Vec::new();

        while input.parse::<Comma>().is_ok() {
            let expr = input.parse::<ExprReference>()?;
            let Expr::Lit(lit) = expr.expr.as_ref() else {
                return Err(syn::Error::new(
                    expr.span(),
                    "example can only handle literal expression",
                ));
            };
            let ty = match lit.lit {
                Lit::Str(_) => quote! { ::xitca_postgres::types::Type::TEXT },
                Lit::Int(_) => quote! { ::xitca_postgres::types::Type::INT4 },
                ref lit => {
                    return Err(syn::Error::new(
                        lit.span(),
                        "example can only handle i32 and String type as parameter",
                    ))
                }
            };
            types.push(ty);
            exprs.push(expr);
        }

        Ok(Self { sql, exprs, types })
    }
}
