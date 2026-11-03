//! Derive macro for `oxide-config`. Use it through `oxide_config::Config` (or `oxide::config`).

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::{Data, DeriveInput, Expr, Fields, Ident, LitStr, parse_macro_input};

/// Declares a typed configuration section.
///
/// ```ignore
/// #[derive(Clone, Deserialize, Config)]
/// #[config(file = "app", validate)]
/// struct AppConfig {
///     name: String,
///     #[config(env = "APP_PORT", default = 3000)]
///     port: u16,
///     #[config(env = "APP_KEY", secret)]
///     key: String,
/// }
/// ```
///
/// The section is read from `config/app.toml`. A field's value comes from, in order: its
/// environment variable (`env`), the file, then its `default`. The derive also writes a `Debug`
/// impl that prints `[REDACTED]` for `secret` fields, so don't derive `Debug` yourself. Add
/// `validate` to the section to run `validator::Validate` on it at boot.
///
/// The struct must implement `serde::Deserialize`.
#[proc_macro_derive(Config, attributes(config))]
pub fn derive_config(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// The path generated code uses for `oxide_config`.
///
/// A crate that depends on `oxide-config` directly gets `::oxide_config` (or its renamed form);
/// apps that depend only on the `oxide` umbrella crate get `::oxide::config`.
fn krate() -> TokenStream2 {
    use proc_macro_crate::{FoundCrate, crate_name};

    let root = |name: &str| Ident::new(name, Span::call_site());
    match crate_name("oxide-config") {
        Ok(FoundCrate::Name(name)) => {
            let name = root(&name);
            quote!(::#name)
        }
        // `oxide-config` itself declares `extern crate self as oxide_config`.
        Ok(FoundCrate::Itself) => quote!(::oxide_config),
        Err(_) => match crate_name("oxide") {
            Ok(FoundCrate::Itself) => quote!(::oxide::config),
            Ok(FoundCrate::Name(name)) => {
                let name = root(&name);
                quote!(::#name::config)
            }
            Err(_) => quote!(::oxide_config),
        },
    }
}

struct SectionAttrs {
    file: LitStr,
    validate: bool,
}

struct FieldAttrs {
    env: Option<LitStr>,
    default: Option<Expr>,
    secret: bool,
}

fn section_attrs(input: &DeriveInput) -> syn::Result<SectionAttrs> {
    let (mut file, mut validate) = (None, false);
    for attr in input.attrs.iter().filter(|a| a.path().is_ident("config")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("file") {
                file = Some(meta.value()?.parse::<LitStr>()?);
            } else if meta.path.is_ident("validate") {
                validate = true;
            } else {
                return Err(meta.error("expected `file = \"...\"` or `validate`"));
            }
            Ok(())
        })?;
    }
    let file = file.ok_or_else(|| {
        syn::Error::new(
            input.ident.span(),
            "`#[derive(Config)]` needs `#[config(file = \"app\")]`: the file under `config/` the section is read from",
        )
    })?;
    Ok(SectionAttrs { file, validate })
}

fn field_attrs(field: &syn::Field) -> syn::Result<FieldAttrs> {
    let mut attrs = FieldAttrs {
        env: None,
        default: None,
        secret: false,
    };
    for attr in field.attrs.iter().filter(|a| a.path().is_ident("config")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("env") {
                attrs.env = Some(meta.value()?.parse::<LitStr>()?);
            } else if meta.path.is_ident("default") {
                attrs.default = Some(meta.value()?.parse::<Expr>()?);
            } else if meta.path.is_ident("secret") {
                attrs.secret = true;
            } else {
                return Err(meta.error("expected `env = \"...\"`, `default = ...` or `secret`"));
            }
            Ok(())
        })?;
    }
    Ok(attrs)
}

fn expand(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let krate = krate();
    let ident = &input.ident;
    let section = section_attrs(input)?;
    let file = &section.file;

    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            ident,
            "`#[derive(Config)]` only supports structs with named fields",
        ));
    };
    let Fields::Named(named) = &data.fields else {
        return Err(syn::Error::new_spanned(
            ident,
            "`#[derive(Config)]` only supports structs with named fields",
        ));
    };

    let mut specs = Vec::new();
    let mut debug_fields = Vec::new();
    for field in &named.named {
        let Some(name) = field.ident.as_ref() else {
            continue;
        };
        let name_str = name.to_string();
        let ty = &field.ty;
        let attrs = field_attrs(field)?;
        let secret = attrs.secret;

        let env = if let Some(env) = &attrs.env {
            quote!(::core::option::Option::Some(#env))
        } else {
            quote!(::core::option::Option::None)
        };
        let coerce = if attrs.env.is_some() {
            quote!(::core::option::Option::Some(
                <#ty as #krate::EnvValue>::from_env as #krate::EnvCoerce
            ))
        } else {
            quote!(::core::option::Option::None)
        };
        let default = if let Some(expr) = &attrs.default {
            quote!(::core::option::Option::Some(
                (|| #krate::__private::to_value(&#expr)) as #krate::DefaultFn
            ))
        } else {
            quote!(::core::option::Option::None)
        };
        specs.push(quote! {
            #krate::FieldSpec {
                name: #name_str,
                env: #env,
                secret: #secret,
                default: #default,
                coerce: #coerce,
            }
        });

        debug_fields.push(if secret {
            quote!(.field(#name_str, &"[REDACTED]"))
        } else {
            quote!(.field(#name_str, &self.#name))
        });
    }

    let check = if section.validate {
        quote! {
            fn check(&self) -> ::core::result::Result<(), ::std::vec::Vec<::std::string::String>> {
                #krate::__private::validate(self)
            }
        }
    } else {
        quote!()
    };

    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let ident_str = ident.to_string();
    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics #krate::ConfigSection for #ident #ty_generics #where_clause {
            const FILE: &'static str = #file;

            fn fields() -> &'static [#krate::FieldSpec] {
                const FIELDS: &[#krate::FieldSpec] = &[#(#specs),*];
                FIELDS
            }

            #check
        }

        #[automatically_derived]
        impl #impl_generics ::core::fmt::Debug for #ident #ty_generics #where_clause {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.debug_struct(#ident_str)
                    #(#debug_fields)*
                    .finish()
            }
        }
    })
}
