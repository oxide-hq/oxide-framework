//! Derive macros for `oxide-http`.
//!
//! These macros are re-exported by `oxide-http`; depend on that crate rather than on this one.
//! All generated code refers to `::oxide_http::__private::*` (or `::oxide::http::__private::*` in
//! apps that depend on the `oxide` umbrella crate) so that user crates do not need direct
//! `axum`/`tokio` dependencies.

use proc_macro::TokenStream;
use proc_macro2::{Ident, Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, parse_macro_input, parse_quote};

mod routes;

/// Lets a service be extracted directly as a handler argument.
///
/// Generates `impl<S> FromRequestParts<S> for T where T: FromRef<S>`, so a handler can take
/// `mailer: Mailer` instead of `State(mailer): State<Mailer>`. Pair it with a state struct that
/// derives [`ServiceContainer`] (or any manual `FromRef` impl).
#[proc_macro_derive(Service)]
pub fn derive_service(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_service(&input).into()
}

/// Turns an application state struct into a service container.
///
/// For every field this generates `impl FromRef<Container> for FieldType`, and it implements
/// `oxide_http::container::ServiceContainer`, backed by a `tokio::task_local!`, so the container
/// can be accessed statically ("facades") while a request is being handled inside
/// `oxide_http::container::facade_middleware`.
#[proc_macro_derive(ServiceContainer)]
pub fn derive_service_container(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_service_container(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Implements `oxide_http::request::FormRequest<S>` for any `S: Send + Sync`.
///
/// Validation is delegated to `validator::Validate`. Authorization defaults to "always allowed";
/// add `#[form_request(authorize)]` to delegate to your own `oxide_http::request::Authorize<S>`
/// implementation instead.
#[proc_macro_derive(FormRequest, attributes(form_request))]
pub fn derive_form_request(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_form_request(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Declares routes with a Laravel-style DSL and expands into an `oxide_http::routing::OxideRouter`.
///
/// ```ignore
/// let router: OxideRouter<AppContext> = routes! {
///     get("/", home).name("home");
///
///     prefix("/api/v1") {
///         get("/health", health_check).name("health");
///         resource("/posts", PostController);                  // index, store, show, update, destroy
///         post("/posts/{post}/publish", PostController::publish).name("posts.publish");
///
///         middleware(axum::middleware::from_fn(require_token)) {
///             delete("/cache", clear_cache).name("cache.clear");
///         }
///     }
/// };
/// ```
///
/// Statements:
/// - `get | post | put | patch | delete | head | options(path, handler) [.name("...")];`
/// - `resource(path, Controller [, param = "post"] [, only = [index, show]] [, except = [destroy]]);`
///   registers `index`/`store` on `path` and `show`/`update`/`destroy` on `path/{param}`, named
///   `"<resource>.<action>"` (PATCH is routed to `update` too). The parameter defaults to the
///   singular of the last path segment (`posts` → `post`).
/// - `prefix(path) [.name("admin.")] { ... }` prefixes paths (and optionally names); nests freely.
/// - `middleware(layer) { ... }` applies a Tower layer to the routes inside.
///
/// Paths must start with `/`, and duplicate route names are compile errors.
#[proc_macro]
pub fn routes(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as routes::Routes);
    routes::expand(&parsed)
        .unwrap_or_else(|error| {
            // `routes!` is an expression: wrap (possibly several) `compile_error!`s in a block.
            let errors = error.into_compile_error();
            quote! {{ #errors ::core::unreachable!() }}
        })
        .into()
}

fn expand_service(input: &DeriveInput) -> TokenStream2 {
    let krate = krate();
    let ident = &input.ident;
    let (_, ty_generics, _) = input.generics.split_for_impl();

    let mut generics = input.generics.clone();
    generics.params.push(parse_quote!(__OxideState));
    let where_clause = generics.make_where_clause();
    where_clause.predicates.push(parse_quote!(
        #ident #ty_generics: #krate::__private::axum::extract::FromRef<__OxideState>
    ));
    where_clause
        .predicates
        .push(parse_quote!(__OxideState: ::core::marker::Send + ::core::marker::Sync));
    where_clause
        .predicates
        .push(parse_quote!(#ident #ty_generics: ::core::marker::Send));
    let (impl_generics, _, where_clause) = generics.split_for_impl();

    quote! {
        #[automatically_derived]
        impl #impl_generics #krate::__private::axum::extract::FromRequestParts<__OxideState>
            for #ident #ty_generics #where_clause
        {
            type Rejection = ::core::convert::Infallible;

            fn from_request_parts(
                _parts: &mut #krate::__private::axum::http::request::Parts,
                state: &__OxideState,
            ) -> impl ::core::future::Future<Output = ::core::result::Result<Self, Self::Rejection>>
                   + ::core::marker::Send {
                // Resolution is synchronous; avoid an `async` state machine.
                ::core::future::ready(::core::result::Result::Ok(
                    <Self as #krate::__private::axum::extract::FromRef<__OxideState>>::from_ref(state),
                ))
            }
        }
    }
}

fn expand_service_container(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let krate = krate();
    let ident = &input.ident;

    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "`ServiceContainer` cannot be derived for generic structs (task-local storage needs a concrete type)",
        ));
    }

    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            ident,
            "`ServiceContainer` can only be derived for structs with named fields",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            ident,
            "`ServiceContainer` can only be derived for structs with named fields",
        ));
    };

    // Two fields of the same type would generate conflicting `FromRef` impls; report it nicely.
    let mut seen: Vec<(String, &syn::Field)> = Vec::new();
    for field in &fields.named {
        let ty = &field.ty;
        let key = quote!(#ty).to_string();
        if let Some((_, first)) = seen.iter().find(|(k, _)| *k == key) {
            let first_name = first
                .ident
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default();
            return Err(syn::Error::new_spanned(
                ty,
                format!(
                    "`ServiceContainer`: type `{key}` is already provided by field `{first_name}`; \
                     each service type may appear only once (wrap it in a newtype)"
                ),
            ));
        }
        seen.push((key, field));
    }

    let from_ref_impls = fields.named.iter().filter_map(|field| {
        let name = field.ident.as_ref()?;
        let ty = &field.ty;
        Some(quote! {
            #[automatically_derived]
            impl #krate::__private::axum::extract::FromRef<#ident> for #ty {
                fn from_ref(container: &#ident) -> Self {
                    ::core::clone::Clone::clone(&container.#name)
                }
            }
        })
    });

    let local = format_ident!("__OXIDE_{}_CONTEXT", ident.to_string().to_uppercase());

    Ok(quote! {
        #(#from_ref_impls)*

        const _: () = {
            #krate::__private::tokio::task_local! {
                static #local: #ident;
            }

            #[automatically_derived]
            impl #krate::container::ServiceContainer for #ident {
                fn scope<F>(
                    self,
                    future: F,
                ) -> #krate::__private::tokio::task::futures::TaskLocalFuture<Self, F>
                where
                    F: ::core::future::Future,
                {
                    #local.scope(self, future)
                }

                fn try_current() -> ::core::option::Option<Self> {
                    #local.try_with(::core::clone::Clone::clone).ok()
                }
            }
        };
    })
}

fn expand_form_request(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let krate = krate();
    let ident = &input.ident;
    let custom_authorize = parse_form_request_attrs(input)?;
    let (_, ty_generics, _) = input.generics.split_for_impl();

    let mut generics = input.generics.clone();
    generics.params.push(parse_quote!(__OxideState));
    let where_clause = generics.make_where_clause();
    where_clause
        .predicates
        .push(parse_quote!(__OxideState: ::core::marker::Send + ::core::marker::Sync));
    if custom_authorize {
        where_clause.predicates.push(parse_quote!(
            #ident #ty_generics: #krate::request::Authorize<__OxideState>
        ));
    }
    let (impl_generics, _, where_clause) = generics.split_for_impl();

    let authorize = custom_authorize.then(|| {
        quote! {
            fn authorize(
                &self,
                parts: &#krate::__private::axum::http::request::Parts,
                state: &__OxideState,
            ) -> impl ::core::future::Future<Output = bool> + ::core::marker::Send {
                <Self as #krate::request::Authorize<__OxideState>>::authorize(self, parts, state)
            }
        }
    });

    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics #krate::request::FormRequest<__OxideState>
            for #ident #ty_generics #where_clause
        {
            #authorize

            fn validate_fields(
                &self,
            ) -> ::core::result::Result<(), #krate::__private::validator::ValidationErrors> {
                <Self as #krate::__private::validator::Validate>::validate(self)
            }
        }
    })
}

/// The path generated code uses for `oxide_http`.
///
/// A crate that depends on `oxide-http` directly gets `::oxide_http` (or its renamed form). Apps that
/// depend only on the `oxide` umbrella crate get `::oxide::http`.
pub(crate) fn krate() -> TokenStream2 {
    use proc_macro_crate::{FoundCrate, crate_name};

    let root = |name: &str| Ident::new(name, Span::call_site());
    // A direct dependency wins: it is always usable unless it is optional and disabled, whereas
    // `oxide` may be present only as a disabled optional dependency of an `oxide-http` user.
    match crate_name("oxide-http") {
        Ok(FoundCrate::Name(name)) => {
            let name = root(&name);
            quote!(::#name)
        }
        // `oxide-http` itself declares `extern crate self as oxide_http`.
        Ok(FoundCrate::Itself) => quote!(::oxide_http),
        Err(_) => match crate_name("oxide") {
            Ok(FoundCrate::Itself) => quote!(::oxide::http),
            Ok(FoundCrate::Name(name)) => {
                let name = root(&name);
                quote!(::#name::http)
            }
            Err(_) => quote!(::oxide_http),
        },
    }
}

/// Parses `#[form_request(authorize)]` (alias: `custom_authorize`).
fn parse_form_request_attrs(input: &DeriveInput) -> syn::Result<bool> {
    let mut custom_authorize = false;
    for attr in input
        .attrs
        .iter()
        .filter(|a| a.path().is_ident("form_request"))
    {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("authorize") || meta.path.is_ident("custom_authorize") {
                custom_authorize = true;
                Ok(())
            } else {
                Err(meta.error("unknown `form_request` option; expected `authorize`"))
            }
        })?;
    }
    Ok(custom_authorize)
}
