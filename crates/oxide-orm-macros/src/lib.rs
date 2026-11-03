//! `#[derive(Model)]` for `oxide-orm`. Depend on `oxide-orm`, which re-exports it.

use std::collections::HashMap;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    Data, DeriveInput, Fields, GenericArgument, Ident, LitStr, PathArguments, Type, Visibility,
    parse_macro_input,
};

/// Derives `oxide_orm::Model` plus a typed query builder, active-record methods and route-model
/// binding.
///
/// ```ignore
/// #[derive(Debug, Clone, Serialize, sqlx::FromRow, Model)]
/// #[model(table = "posts")]            // default: snake_case plural of the type name
/// pub struct Post {
///     #[model(primary_key)]            // default: the field named `id`
///     pub id: i64,
///     pub title: String,
///     pub published: bool,
/// }
/// ```
///
/// Generates `Post::query()`, `find`, `find_or_fail`, `all`, `insert(&db, &NewPost { .. })`,
/// `create`, `save`, `delete`, a `PostQueryBuilder` with `where_<field>[_not|_in]` and
/// `order_by_<field>_{asc,desc}` for every field (plus `_contains`/`_starts_with`/`_ends_with` on
/// `String`, `_gt`/`_gte`/`_lt`/`_lte` on numbers, `_null`/`_not_null` on `Option`), a `NewPost`
/// struct (all fields except an integer primary key) and `RouteBindable<S>` for every state `S`
/// that provides a `DatabasePool`. Struct options: `table = "..."`, `route_param = "..."`.
#[proc_macro_derive(Model, attributes(model))]
pub fn derive_model(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Text,
    Numeric,
    Integer,
    Other,
}

struct Column<'a> {
    ident: &'a Ident,
    name: String,
    ty: &'a Type,
    /// `T` for `Option<T>` fields, otherwise the field type.
    value_ty: &'a Type,
    optional: bool,
    kind: Kind,
}

fn expand(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let ident = &input.ident;
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "`Model` cannot be derived for generic structs",
        ));
    }
    let (table, route_param) = parse_struct_attrs(input)?;
    let table = table.unwrap_or_else(|| pluralize(&snake_case(&ident.to_string())));
    let route_param = route_param.unwrap_or_else(|| snake_case(&ident.to_string()));

    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            ident,
            "`Model` can only be derived for structs",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            ident,
            "`Model` requires a struct with named fields",
        ));
    };

    let rename_all = sqlx_rename_all(input)?;
    let (columns, pk_index) = parse_columns(ident, fields, rename_all.as_deref())?;
    let pk = &columns[pk_index];
    if pk.optional {
        return Err(syn::Error::new_spanned(
            pk.ty,
            "the primary key cannot be an `Option`",
        ));
    }

    let vis = &input.vis;
    let builder = format_ident!("{}QueryBuilder", ident);
    let new_struct = format_ident!("New{}", ident);
    let name = ident.to_string();

    let model_impl = model_impl(ident, &name, &table, &columns, pk, &builder);
    let builder_def = builder_def(vis, ident, &builder, &columns)?;
    let (new_def, insert_values) = new_struct_def(vis, ident, &new_struct, &columns, pk);
    let pk_ty = pk.ty;
    let binding_impl = binding_impl(ident, pk_ty, &route_param);

    Ok(quote! {
        #model_impl
        #builder_def
        #new_def

        #[automatically_derived]
        impl #ident {
            /// Starts a typed query.
            #vis fn query() -> #builder {
                #builder::new()
            }

            /// Finds a row by primary key.
            #vis async fn find(
                db: &::oxide_orm::DatabasePool,
                id: #pk_ty,
            ) -> ::core::result::Result<::core::option::Option<Self>, ::oxide_orm::OrmError> {
                ::oxide_orm::model::find::<Self>(db, id).await
            }

            /// Finds a row by primary key, or fails with `OrmError::NotFound` (a 404 over HTTP).
            #vis async fn find_or_fail(
                db: &::oxide_orm::DatabasePool,
                id: #pk_ty,
            ) -> ::core::result::Result<Self, ::oxide_orm::OrmError> {
                ::oxide_orm::model::find_or_fail::<Self>(db, id).await
            }

            /// Every row.
            #vis async fn all(
                db: &::oxide_orm::DatabasePool,
            ) -> ::core::result::Result<::std::vec::Vec<Self>, ::oxide_orm::OrmError> {
                #builder::new().get(db).await
            }

            /// Inserts a row and returns it as stored (`RETURNING *`).
            #vis async fn insert(
                db: &::oxide_orm::DatabasePool,
                new: &#new_struct,
            ) -> ::core::result::Result<Self, ::oxide_orm::OrmError> {
                ::oxide_orm::model::insert::<Self>(db, #insert_values).await
            }

            /// Inserts a row and returns it as stored; by-value form of `insert`.
            #vis async fn create(
                db: &::oxide_orm::DatabasePool,
                new: #new_struct,
            ) -> ::core::result::Result<Self, ::oxide_orm::OrmError> {
                Self::insert(db, &new).await
            }

            /// Writes every column back to this row (`UPDATE ... WHERE <pk> = ?`).
            #vis async fn save(
                &self,
                db: &::oxide_orm::DatabasePool,
            ) -> ::core::result::Result<(), ::oxide_orm::OrmError> {
                ::oxide_orm::model::save(db, self).await
            }

            /// Deletes this row.
            #vis async fn delete(
                &self,
                db: &::oxide_orm::DatabasePool,
            ) -> ::core::result::Result<(), ::oxide_orm::OrmError> {
                ::oxide_orm::model::delete(db, self).await
            }
        }

        #binding_impl
    })
}

fn parse_columns<'a>(
    ident: &Ident,
    fields: &'a syn::FieldsNamed,
    rename_all: Option<&str>,
) -> syn::Result<(Vec<Column<'a>>, usize)> {
    let mut columns = Vec::new();
    let mut pk_index = None;
    let mut default_pk = None;
    for field in &fields.named {
        let field_ident = field.ident.as_ref().expect("named field");
        if parse_field_attrs(field)? {
            if pk_index.is_some() {
                return Err(syn::Error::new_spanned(
                    field,
                    "only one field can be `#[model(primary_key)]`",
                ));
            }
            pk_index = Some(columns.len());
        }
        let field_name = field_ident.to_string().trim_start_matches("r#").to_owned();
        // Column names follow sqlx's `FromRow` mapping so queries hit the columns it reads.
        let name = match sqlx_rename(field)? {
            Some(name) => name,
            None => {
                rename_all.map_or_else(|| field_name.clone(), |case| apply_case(&field_name, case))
            }
        };
        if field_name == "id" {
            default_pk = Some(columns.len());
        }
        let (value_ty, optional) =
            option_inner(&field.ty).map_or((&field.ty, false), |inner| (inner, true));
        columns.push(Column {
            ident: field_ident,
            name,
            ty: &field.ty,
            value_ty,
            optional,
            kind: kind_of(value_ty),
        });
    }
    let pk_index = pk_index.or(default_pk).ok_or_else(|| {
        syn::Error::new_spanned(
            ident,
            "`Model` needs a primary key: name a field `id` or mark one `#[model(primary_key)]`",
        )
    })?;
    Ok((columns, pk_index))
}

/// `RouteBindable<S>` for every state `S` that provides a `DatabasePool`.
fn binding_impl(ident: &Ident, pk_ty: &Type, route_param: &str) -> TokenStream2 {
    quote! {
        #[automatically_derived]
        impl<S> ::oxide_orm::__private::oxide_http::binding::RouteBindable<S> for #ident
        where
            S: ::core::marker::Send + ::core::marker::Sync,
            ::oxide_orm::DatabasePool: ::oxide_orm::__private::axum::extract::FromRef<S>,
        {
            type Id = #pk_ty;

            fn param_name() -> &'static str {
                #route_param
            }

            fn bind_from_state(
                id: Self::Id,
                state: &S,
            ) -> impl ::core::future::Future<
                Output = ::core::result::Result<
                    ::core::option::Option<Self>,
                    ::oxide_orm::__private::oxide_http::OxideHttpError,
                >,
            > + ::core::marker::Send {
                let db = <::oxide_orm::DatabasePool as ::oxide_orm::__private::axum::extract::FromRef<S>>::from_ref(state);
                async move {
                    Self::find(&db, id).await.map_err(::core::convert::Into::into)
                }
            }
        }
    }
}

fn model_impl(
    ident: &Ident,
    name: &str,
    table: &str,
    columns: &[Column<'_>],
    pk: &Column<'_>,
    builder: &Ident,
) -> TokenStream2 {
    let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    let pk_name = &pk.name;
    let pk_ident = pk.ident;
    let pk_ty = pk.ty;
    let value_arms = columns.iter().map(|c| {
        let (col, field) = (&c.name, c.ident);
        quote!(#col => ::core::option::Option::Some(::oxide_orm::SqlValue::from(::core::clone::Clone::clone(&self.#field))))
    });
    let non_pk_values = columns.iter().filter(|c| c.name != pk.name).map(|c| {
        let (col, field) = (&c.name, c.ident);
        quote!((#col, ::oxide_orm::SqlValue::from(::core::clone::Clone::clone(&self.#field))))
    });

    quote! {
        #[automatically_derived]
        impl ::oxide_orm::Model for #ident {
            const NAME: &'static str = #name;
            const TABLE: &'static str = #table;
            const PRIMARY_KEY: &'static str = #pk_name;
            const COLUMNS: &'static [&'static str] = &[#(#names),*];

            type Key = #pk_ty;
            type QueryBuilder = #builder;

            fn key(&self) -> Self::Key {
                ::core::clone::Clone::clone(&self.#pk_ident)
            }

            fn value_of(&self, column: &str) -> ::core::option::Option<::oxide_orm::SqlValue> {
                match column {
                    #(#value_arms,)*
                    _ => ::core::option::Option::None,
                }
            }

            fn column_values(&self) -> ::std::vec::Vec<(&'static str, ::oxide_orm::SqlValue)> {
                ::std::vec![#(#non_pk_values),*]
            }
        }
    }
}

fn builder_def(
    vis: &Visibility,
    model: &Ident,
    builder: &Ident,
    columns: &[Column<'_>],
) -> syn::Result<TokenStream2> {
    let mut generated = Vec::new();
    let methods: Vec<_> = columns
        .iter()
        .map(|column| column_methods(column, &mut generated))
        .collect();
    reject_collisions(&generated)?;
    let doc = format!("Typed query builder for [`{model}`], generated by `#[derive(Model)]`.");

    Ok(quote! {
        #[doc = #doc]
        #[derive(Debug, Clone)]
        #vis struct #builder {
            query: ::oxide_orm::Query<#model>,
        }

        #[automatically_derived]
        impl ::core::convert::From<::oxide_orm::Query<#model>> for #builder {
            fn from(query: ::oxide_orm::Query<#model>) -> Self {
                Self { query }
            }
        }

        #[automatically_derived]
        impl ::core::default::Default for #builder {
            fn default() -> Self {
                Self::new()
            }
        }

        #[automatically_derived]
        #[allow(dead_code)]
        impl #builder {
            #vis fn new() -> Self {
                Self { query: ::oxide_orm::Query::new() }
            }

            #(#methods)*

            #[must_use]
            #vis fn limit(self, limit: u32) -> Self {
                Self { query: self.query.limit(limit) }
            }

            #[must_use]
            #vis fn offset(self, offset: u32) -> Self {
                Self { query: self.query.offset(offset) }
            }

            /// The underlying untyped query.
            #vis fn into_query(self) -> ::oxide_orm::Query<#model> {
                self.query
            }

            /// The `SELECT` statement and bound values.
            #vis fn to_sql(&self) -> (::std::string::String, ::std::vec::Vec<::oxide_orm::SqlValue>) {
                self.query.to_sql()
            }

            #vis async fn get(
                self,
                db: &::oxide_orm::DatabasePool,
            ) -> ::core::result::Result<::std::vec::Vec<#model>, ::oxide_orm::OrmError> {
                self.query.get(db).await
            }

            #vis async fn first(
                self,
                db: &::oxide_orm::DatabasePool,
            ) -> ::core::result::Result<::core::option::Option<#model>, ::oxide_orm::OrmError> {
                self.query.first(db).await
            }

            #vis async fn first_or_fail(
                self,
                db: &::oxide_orm::DatabasePool,
            ) -> ::core::result::Result<#model, ::oxide_orm::OrmError> {
                self.query.first_or_fail(db).await
            }

            #vis async fn count(
                self,
                db: &::oxide_orm::DatabasePool,
            ) -> ::core::result::Result<i64, ::oxide_orm::OrmError> {
                self.query.count(db).await
            }

            /// Deletes the matching rows and returns how many were removed.
            #vis async fn delete(
                self,
                db: &::oxide_orm::DatabasePool,
            ) -> ::core::result::Result<u64, ::oxide_orm::OrmError> {
                self.query.delete(db).await
            }
        }
    })
}

/// Fails when two fields generate the same method (`score_gt` vs `score` + `_gt`).
fn reject_collisions(generated: &[(Ident, &Ident)]) -> syn::Result<()> {
    let mut seen: HashMap<String, &Ident> = HashMap::new();
    for (method, field) in generated {
        if let Some(first) = seen.insert(method.to_string(), field) {
            return Err(syn::Error::new(
                field.span(),
                format!(
                    "fields `{first}` and `{field}` both generate `{method}`; rename one field \
                     (`#[sqlx(rename = \"...\")]` keeps its column name)"
                ),
            ));
        }
    }
    Ok(())
}

/// `where_*` / `order_by_*` methods for one column.
///
/// Every generated name is recorded in `generated` with its field, for collision checks.
fn column_methods<'a>(
    column: &Column<'a>,
    generated: &mut Vec<(Ident, &'a Ident)>,
) -> TokenStream2 {
    let field = column.ident.to_string();
    let field = field.trim_start_matches("r#");
    let col = &column.name;
    let ty = column.value_ty;
    let span = column.ident.span();
    // `where_title`, `where_title_not`, `order_by_title_asc`, ...
    let mut name = |prefix: &str, suffix: &str| {
        let method = Ident::new(&format!("{prefix}_{field}{suffix}"), span);
        generated.push((method.clone(), column.ident));
        method
    };
    let value = quote!(::oxide_orm::SqlValue::from(::core::convert::Into::<#ty>::into(value)));

    let (eq, not, is_in) = (
        name("where", ""),
        name("where", "_not"),
        name("where", "_in"),
    );
    let (asc, desc) = (name("order_by", "_asc"), name("order_by", "_desc"));
    let mut methods = quote! {
        #[must_use]
        pub fn #eq(self, value: impl ::core::convert::Into<#ty>) -> Self {
            Self { query: self.query.filter(#col, ::oxide_orm::Op::Eq, #value) }
        }

        #[must_use]
        pub fn #not(self, value: impl ::core::convert::Into<#ty>) -> Self {
            Self { query: self.query.filter(#col, ::oxide_orm::Op::Ne, #value) }
        }

        #[must_use]
        pub fn #is_in(self, values: impl ::core::iter::IntoIterator<Item = #ty>) -> Self {
            Self { query: self.query.filter_in(#col, values) }
        }

        #[must_use]
        pub fn #asc(self) -> Self {
            Self { query: self.query.order_by(#col, ::oxide_orm::Direction::Asc) }
        }

        #[must_use]
        pub fn #desc(self) -> Self {
            Self { query: self.query.order_by(#col, ::oxide_orm::Direction::Desc) }
        }
    };

    if column.optional {
        let (null, not_null) = (name("where", "_null"), name("where", "_not_null"));
        methods.extend(quote! {
            #[must_use]
            pub fn #null(self) -> Self {
                Self { query: self.query.filter_null(#col, false) }
            }

            #[must_use]
            pub fn #not_null(self) -> Self {
                Self { query: self.query.filter_null(#col, true) }
            }
        });
    }

    match column.kind {
        Kind::Text => {
            for (suffix, prefix, postfix) in [
                ("_contains", "%", "%"),
                ("_starts_with", "", "%"),
                ("_ends_with", "%", ""),
            ] {
                let method = name("where", suffix);
                methods.extend(quote! {
                    #[must_use]
                    pub fn #method(self, text: &str) -> Self {
                        Self {
                            query: self.query.filter(
                                #col,
                                ::oxide_orm::Op::Like,
                                ::oxide_orm::query::like_pattern(#prefix, text, #postfix),
                            ),
                        }
                    }
                });
            }
        }
        Kind::Numeric | Kind::Integer => {
            for (suffix, op) in [
                ("_gt", quote!(Gt)),
                ("_gte", quote!(Gte)),
                ("_lt", quote!(Lt)),
                ("_lte", quote!(Lte)),
            ] {
                let method = name("where", suffix);
                methods.extend(quote! {
                    #[must_use]
                    pub fn #method(self, value: #ty) -> Self {
                        Self { query: self.query.filter(#col, ::oxide_orm::Op::#op, ::oxide_orm::SqlValue::from(value)) }
                    }
                });
            }
        }
        Kind::Other => {}
    }
    methods
}

/// The `New<Model>` struct and the expression building its `(column, value)` list.
fn new_struct_def(
    vis: &Visibility,
    model: &Ident,
    new_struct: &Ident,
    columns: &[Column<'_>],
    pk: &Column<'_>,
) -> (TokenStream2, TokenStream2) {
    // Integer keys are assigned by SQLite (`INTEGER PRIMARY KEY`); other keys are caller-supplied.
    let skip_pk = pk.kind == Kind::Integer;
    let included: Vec<&Column<'_>> = columns
        .iter()
        .filter(|c| !(skip_pk && c.name == pk.name))
        .collect();
    let fields = included.iter().map(|c| {
        let (field, ty) = (c.ident, c.ty);
        quote!(pub #field: #ty)
    });
    let values = included.iter().map(|c| {
        let (col, field) = (&c.name, c.ident);
        quote!((#col, ::oxide_orm::SqlValue::from(::core::clone::Clone::clone(&new.#field))))
    });
    let doc =
        format!("A new [`{model}`] row for `{model}::insert`, generated by `#[derive(Model)]`.");
    (
        quote! {
            #[doc = #doc]
            #[derive(Debug, Clone)]
            #vis struct #new_struct {
                #(#fields,)*
            }
        },
        quote!(::std::vec![#(#values),*]),
    )
}

/// Parses `#[model(table = "...", route_param = "...")]` on the struct.
fn parse_struct_attrs(input: &DeriveInput) -> syn::Result<(Option<String>, Option<String>)> {
    let (mut table, mut route_param) = (None, None);
    for attr in input.attrs.iter().filter(|a| a.path().is_ident("model")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("table") {
                table = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("route_param") {
                route_param = Some(meta.value()?.parse::<LitStr>()?.value());
            } else {
                return Err(meta.error("unknown `model` option; expected `table` or `route_param`"));
            }
            Ok(())
        })?;
    }
    Ok((table, route_param))
}

/// Returns `true` for `#[model(primary_key)]`.
fn parse_field_attrs(field: &syn::Field) -> syn::Result<bool> {
    let mut primary_key = false;
    for attr in field.attrs.iter().filter(|a| a.path().is_ident("model")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("primary_key") {
                primary_key = true;
                Ok(())
            } else {
                Err(meta.error("unknown `model` field option; expected `primary_key`"))
            }
        })?;
    }
    Ok(primary_key)
}

/// The struct-level `#[sqlx(rename_all = "...")]`, if any.
fn sqlx_rename_all(input: &DeriveInput) -> syn::Result<Option<String>> {
    let mut rename_all = None;
    for attr in input.attrs.iter().filter(|a| a.path().is_ident("sqlx")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename_all") {
                let lit = meta.value()?.parse::<LitStr>()?;
                if !CASES.contains(&lit.value().as_str()) {
                    return Err(syn::Error::new_spanned(
                        &lit,
                        "unsupported `rename_all` case",
                    ));
                }
                rename_all = Some(lit.value());
            } else if meta.input.peek(syn::Token![=]) {
                meta.value()?.parse::<syn::Expr>()?;
            }
            Ok(())
        })?;
    }
    Ok(rename_all)
}

/// A field's `#[sqlx(rename = "...")]`, rejecting `FromRow` options the model cannot mirror.
fn sqlx_rename(field: &syn::Field) -> syn::Result<Option<String>> {
    let mut rename = None;
    for attr in field.attrs.iter().filter(|a| a.path().is_ident("sqlx")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                rename = Some(meta.value()?.parse::<LitStr>()?.value());
                Ok(())
            } else if meta.path.is_ident("default") {
                Ok(())
            } else {
                Err(meta.error(
                    "`Model` supports only `#[sqlx(rename)]` and `#[sqlx(default)]` on fields",
                ))
            }
        })?;
    }
    Ok(rename)
}

/// The `rename_all` cases sqlx supports.
const CASES: &[&str] = &[
    "lowercase",
    "snake_case",
    "UPPERCASE",
    "SCREAMING_SNAKE_CASE",
    "kebab-case",
    "camelCase",
    "PascalCase",
];

/// Converts a `snake_case` field name to `case` (one of [`CASES`]).
fn apply_case(field: &str, case: &str) -> String {
    let capitalize = |word: &str| {
        let mut chars = word.chars();
        chars.next().map_or_else(String::new, |first| {
            first.to_uppercase().chain(chars).collect()
        })
    };
    let words = field.split('_').filter(|w| !w.is_empty());
    match case {
        "lowercase" => field.to_lowercase(),
        "UPPERCASE" | "SCREAMING_SNAKE_CASE" => field.to_uppercase(),
        "kebab-case" => field.replace('_', "-"),
        "camelCase" => words
            .enumerate()
            .map(|(i, w)| if i == 0 { w.to_owned() } else { capitalize(w) })
            .collect(),
        "PascalCase" => words.map(capitalize).collect(),
        _ => field.to_owned(),
    }
}

fn last_segment(ty: &Type) -> Option<&syn::PathSegment> {
    match ty {
        Type::Path(path) if path.qself.is_none() => path.path.segments.last(),
        _ => None,
    }
}

fn option_inner(ty: &Type) -> Option<&Type> {
    let segment = last_segment(ty)?;
    if segment.ident != "Option" {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    match args.args.first()? {
        GenericArgument::Type(inner) => Some(inner),
        _ => None,
    }
}

fn kind_of(ty: &Type) -> Kind {
    let Some(segment) = last_segment(ty) else {
        return Kind::Other;
    };
    match segment.ident.to_string().as_str() {
        "String" => Kind::Text,
        "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" => Kind::Integer,
        "f32" | "f64" => Kind::Numeric,
        _ => Kind::Other,
    }
}

/// `BlogPost` -> `blog_post`.
fn snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (index, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if index > 0 {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// `post` -> `posts`, `category` -> `categories`, `box` -> `boxes`.
fn pluralize(word: &str) -> String {
    if let Some(stem) = word
        .strip_suffix('y')
        .filter(|stem| !stem.ends_with(['a', 'e', 'i', 'o', 'u']))
    {
        format!("{stem}ies")
    } else if ["s", "x", "z", "ch", "sh"]
        .iter()
        .any(|s| word.ends_with(s))
    {
        format!("{word}es")
    } else {
        format!("{word}s")
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_case, pluralize, snake_case};

    #[test]
    fn names() {
        assert_eq!(snake_case("BlogPost"), "blog_post");
        assert_eq!(snake_case("Post"), "post");
        assert_eq!(pluralize("post"), "posts");
        assert_eq!(pluralize("category"), "categories");
        assert_eq!(pluralize("day"), "days");
        assert_eq!(pluralize("box"), "boxes");
        assert_eq!(pluralize("blog_post"), "blog_posts");
    }

    #[test]
    fn applies_sqlx_rename_all_cases() {
        assert_eq!(apply_case("author_id", "camelCase"), "authorId");
        assert_eq!(apply_case("author_id", "PascalCase"), "AuthorId");
        assert_eq!(apply_case("author_id", "SCREAMING_SNAKE_CASE"), "AUTHOR_ID");
        assert_eq!(apply_case("author_id", "kebab-case"), "author-id");
        assert_eq!(apply_case("author_id", "lowercase"), "author_id");
        assert_eq!(apply_case("author_id", "UPPERCASE"), "AUTHOR_ID");
        assert_eq!(apply_case("author_id", "snake_case"), "author_id");
    }
}
