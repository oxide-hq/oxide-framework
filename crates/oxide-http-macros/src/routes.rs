//! The `routes!` DSL: parsing and expansion into `oxide_http::routing::OxideRouter` calls.
//!
//! Prefix blocks are flattened at compile time (each route is registered with its full path), so
//! the expansion never nests Axum routers. Middleware blocks build a child router, apply the
//! layer, and merge it back.

use std::collections::HashMap;

use proc_macro2::{Span, TokenStream};
use quote::{quote, quote_spanned};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Expr, Ident, LitStr, Path, Token, braced, bracketed, parenthesized};

const VERBS: &[&str] = &["get", "post", "put", "patch", "delete", "head", "options"];
const RESOURCE_ACTIONS: &[&str] = &["index", "store", "show", "update", "destroy"];

/// The whole `routes! { ... }` body.
pub struct Routes {
    stmts: Vec<Stmt>,
}

enum Stmt {
    Route {
        verb: Ident,
        path: LitStr,
        handler: Expr,
        name: Option<LitStr>,
    },
    Resource(Resource),
    Prefix {
        path: LitStr,
        name: Option<LitStr>,
        body: Vec<Stmt>,
    },
    Middleware {
        layer: Expr,
        body: Vec<Stmt>,
    },
}

struct Resource {
    keyword: Ident,
    path: LitStr,
    controller: Path,
    param: Option<LitStr>,
    only: Option<Vec<Ident>>,
    except: Option<Vec<Ident>>,
}

impl Parse for Routes {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        Ok(Self {
            stmts: parse_stmts(input)?,
        })
    }
}

fn parse_stmts(input: ParseStream<'_>) -> syn::Result<Vec<Stmt>> {
    let mut stmts = Vec::new();
    while !input.is_empty() {
        stmts.push(input.parse()?);
    }
    Ok(stmts)
}

impl Parse for Stmt {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let keyword: Ident = input.parse()?;
        let kw = keyword.to_string();
        let args;
        parenthesized!(args in input);

        if VERBS.contains(&kw.as_str()) {
            let path: LitStr = args.parse()?;
            args.parse::<Token![,]>()?;
            let handler: Expr = args.parse()?;
            args.parse::<Option<Token![,]>>()?;
            ensure_empty(&args)?;
            let name = parse_name_suffix(input)?;
            input.parse::<Token![;]>()?;
            return Ok(Self::Route {
                verb: keyword,
                path,
                handler,
                name,
            });
        }

        match kw.as_str() {
            "resource" => {
                let resource = parse_resource(keyword, &args)?;
                input.parse::<Token![;]>()?;
                Ok(Self::Resource(resource))
            }
            "prefix" => {
                let path: LitStr = args.parse()?;
                ensure_empty(&args)?;
                let name = parse_name_suffix(input)?;
                let body = parse_block(input)?;
                Ok(Self::Prefix { path, name, body })
            }
            "middleware" => {
                let layer: Expr = args.parse()?;
                args.parse::<Option<Token![,]>>()?;
                ensure_empty(&args)?;
                let body = parse_block(input)?;
                Ok(Self::Middleware { layer, body })
            }
            _ => Err(syn::Error::new(
                keyword.span(),
                format!(
                    "unknown routes! statement `{kw}`; expected one of: {}, resource, prefix, middleware",
                    VERBS.join(", ")
                ),
            )),
        }
    }
}

fn ensure_empty(args: ParseStream<'_>) -> syn::Result<()> {
    if args.is_empty() {
        Ok(())
    } else {
        Err(args.error("unexpected tokens"))
    }
}

/// Parses an optional `.name("...")` suffix.
fn parse_name_suffix(input: ParseStream<'_>) -> syn::Result<Option<LitStr>> {
    if !input.peek(Token![.]) {
        return Ok(None);
    }
    input.parse::<Token![.]>()?;
    let method: Ident = input.parse()?;
    if method != "name" {
        return Err(syn::Error::new(method.span(), "expected `.name(\"...\")`"));
    }
    let args;
    parenthesized!(args in input);
    let name: LitStr = args.parse()?;
    ensure_empty(&args)?;
    Ok(Some(name))
}

/// Parses `{ stmts }` with an optional trailing `;`.
fn parse_block(input: ParseStream<'_>) -> syn::Result<Vec<Stmt>> {
    let body;
    braced!(body in input);
    let stmts = parse_stmts(&body)?;
    input.parse::<Option<Token![;]>>()?;
    Ok(stmts)
}

/// `resource("/posts", PostController [, param = "post"] [, only = [..]] [, except = [..]])`
fn parse_resource(keyword: Ident, args: ParseStream<'_>) -> syn::Result<Resource> {
    let path: LitStr = args.parse()?;
    args.parse::<Token![,]>()?;
    let controller: Path = args.parse()?;
    let mut resource = Resource {
        keyword,
        path,
        controller,
        param: None,
        only: None,
        except: None,
    };

    while args.parse::<Option<Token![,]>>()?.is_some() && !args.is_empty() {
        let option: Ident = args.parse()?;
        args.parse::<Token![=]>()?;
        match option.to_string().as_str() {
            "param" => resource.param = Some(args.parse()?),
            "only" | "except" => {
                let list;
                bracketed!(list in args);
                let actions: Vec<Ident> = Punctuated::<Ident, Token![,]>::parse_terminated(&list)?
                    .into_iter()
                    .collect();
                for action in &actions {
                    if !RESOURCE_ACTIONS.contains(&action.to_string().as_str()) {
                        return Err(syn::Error::new(
                            action.span(),
                            format!(
                                "unknown resource action `{action}`; expected one of: {}",
                                RESOURCE_ACTIONS.join(", ")
                            ),
                        ));
                    }
                }
                if option == "only" {
                    resource.only = Some(actions);
                } else {
                    resource.except = Some(actions);
                }
            }
            other => {
                return Err(syn::Error::new(
                    option.span(),
                    format!(
                        "unknown resource option `{other}`; expected `param`, `only` or `except`"
                    ),
                ));
            }
        }
    }
    ensure_empty(args)?;

    if resource.only.is_some() && resource.except.is_some() {
        return Err(syn::Error::new(
            resource.keyword.span(),
            "use either `only` or `except`, not both",
        ));
    }
    Ok(resource)
}

/// Path/name prefixes accumulated from enclosing `prefix` blocks.
#[derive(Clone, Default)]
struct Scope {
    path: String,
    name: String,
}

impl Scope {
    fn nested(&self, path: &LitStr, name: Option<&LitStr>) -> syn::Result<Self> {
        let segment = checked_path(path)?;
        Ok(Self {
            path: join(&self.path, segment.trim_end_matches('/')),
            name: format!(
                "{}{}",
                self.name,
                name.map(LitStr::value).unwrap_or_default()
            ),
        })
    }
}

struct Expander {
    /// Route name -> span of its first definition, for duplicate detection.
    names: HashMap<String, Span>,
}

impl Expander {
    fn block(&mut self, stmts: &[Stmt], scope: &Scope) -> syn::Result<TokenStream> {
        let mut calls = Vec::new();
        self.collect(stmts, scope, &mut calls)?;
        Ok(quote! {{
            let router = ::oxide_http::routing::OxideRouter::new();
            #( let router = router #calls; )*
            router
        }})
    }

    /// Emits `.route_named(..)` / `.merge(..)` calls for `stmts`, flattening prefix blocks.
    fn collect(
        &mut self,
        stmts: &[Stmt],
        scope: &Scope,
        calls: &mut Vec<TokenStream>,
    ) -> syn::Result<()> {
        for stmt in stmts {
            match stmt {
                Stmt::Route {
                    verb,
                    path,
                    handler,
                    name,
                } => {
                    let full = join(&scope.path, &checked_path(path)?);
                    let name = match name {
                        Some(lit) => {
                            Some(self.claim(format!("{}{}", scope.name, lit.value()), lit.span())?)
                        }
                        None => None,
                    };
                    calls.push(route_call(
                        verb,
                        &full,
                        handler,
                        name.as_deref(),
                        path.span(),
                    ));
                }
                Stmt::Resource(resource) => self.resource(resource, scope, calls)?,
                Stmt::Prefix { path, name, body } => {
                    self.collect(body, &scope.nested(path, name.as_ref())?, calls)?;
                }
                Stmt::Middleware { layer, body } => {
                    let child = self.block(body, scope)?;
                    calls.push(quote_spanned! {layer.span()=>
                        .merge(#child.layer(#layer))
                    });
                }
            }
        }
        Ok(())
    }

    fn resource(
        &mut self,
        resource: &Resource,
        scope: &Scope,
        calls: &mut Vec<TokenStream>,
    ) -> syn::Result<()> {
        let path = checked_path(&resource.path)?;
        let path = path.trim_end_matches('/');
        let statics: Vec<&str> = path
            .split('/')
            .filter(|segment| !segment.is_empty() && !segment.starts_with('{'))
            .collect();
        let Some(last) = statics.last() else {
            return Err(syn::Error::new(
                resource.path.span(),
                "resource path needs at least one static segment, e.g. \"/posts\"",
            ));
        };
        let base_name = format!("{}{}", scope.name, statics.join("."));
        let param = resource
            .param
            .as_ref()
            .map_or_else(|| singularize(last), LitStr::value);
        let collection = join(&scope.path, path);
        let member = format!("{collection}/{{{param}}}");

        let enabled = |action: &str| match (&resource.only, &resource.except) {
            (Some(only), _) => only.iter().any(|a| a == action),
            (_, Some(except)) => !except.iter().any(|a| a == action),
            _ => true,
        };

        let controller = &resource.controller;
        let span = resource.keyword.span();
        for (action, verb, url) in [
            ("index", "get", &collection),
            ("store", "post", &collection),
            ("show", "get", &member),
            ("update", "put", &member),
            ("destroy", "delete", &member),
        ] {
            if !enabled(action) {
                continue;
            }
            let action_ident = Ident::new(action, span);
            let handler: Expr = syn::parse_quote_spanned!(span=> #controller::#action_ident);
            let name = self.claim(format!("{base_name}.{action}"), resource.path.span())?;
            calls.push(route_call(
                &Ident::new(verb, span),
                url,
                &handler,
                Some(&name),
                span,
            ));
            if action == "update" {
                // Laravel routes PATCH to `update` as well.
                calls.push(route_call(
                    &Ident::new("patch", span),
                    url,
                    &handler,
                    None,
                    span,
                ));
            }
        }
        Ok(())
    }

    fn claim(&mut self, name: String, span: Span) -> syn::Result<String> {
        if self.names.contains_key(&name) {
            let mut error = syn::Error::new(span, format!("duplicate route name `{name}`"));
            error.combine(syn::Error::new(self.names[&name], "first defined here"));
            return Err(error);
        }
        self.names.insert(name.clone(), span);
        Ok(name)
    }
}

fn route_call(
    verb: &Ident,
    path: &str,
    handler: &Expr,
    name: Option<&str>,
    span: Span,
) -> TokenStream {
    let method = Ident::new(&verb.to_string().to_uppercase(), verb.span());
    let name = name.map_or_else(
        || quote!(::core::option::Option::None),
        |name| quote!(::core::option::Option::Some(#name)),
    );
    quote_spanned! {span=>
        .route_named(
            ::oxide_http::__private::axum::http::Method::#method,
            #path,
            #handler,
            #name,
        )
    }
}

/// Validates that a route path literal starts with `/` and returns its value.
fn checked_path(lit: &LitStr) -> syn::Result<String> {
    let value = lit.value();
    if value.starts_with('/') {
        Ok(value)
    } else {
        Err(syn::Error::new(
            lit.span(),
            "route paths must start with `/`",
        ))
    }
}

fn join(prefix: &str, path: &str) -> String {
    match (prefix, path) {
        ("", "") => "/".to_owned(),
        (_, "" | "/") if !prefix.is_empty() => prefix.to_owned(),
        _ => format!("{prefix}{path}"),
    }
}

/// `posts` -> `post`, `categories` -> `category`, `boxes` -> `box`, `blog-posts` -> `blog_post`.
fn singularize(segment: &str) -> String {
    let word = segment.replace('-', "_");
    if let Some(stem) = word.strip_suffix("ies") {
        format!("{stem}y")
    } else if ["sses", "shes", "ches", "xes", "zes"]
        .iter()
        .any(|s| word.ends_with(s))
    {
        word[..word.len() - 2].to_owned()
    } else if let Some(stem) = word.strip_suffix('s').filter(|_| !word.ends_with("ss")) {
        stem.to_owned()
    } else {
        word
    }
}

pub fn expand(routes: &Routes) -> syn::Result<TokenStream> {
    let mut expander = Expander {
        names: HashMap::new(),
    };
    expander.block(&routes.stmts, &Scope::default())
}

#[cfg(test)]
mod tests {
    use super::{join, singularize};

    #[test]
    fn singularizes_common_plurals() {
        assert_eq!(singularize("posts"), "post");
        assert_eq!(singularize("categories"), "category");
        assert_eq!(singularize("boxes"), "box");
        assert_eq!(singularize("addresses"), "address");
        assert_eq!(singularize("blog-posts"), "blog_post");
        assert_eq!(singularize("class"), "class");
    }

    #[test]
    fn joins_prefixes() {
        assert_eq!(join("", "/posts"), "/posts");
        assert_eq!(join("/api/v1", "/posts"), "/api/v1/posts");
        assert_eq!(join("/api", "/"), "/api");
        assert_eq!(join("", ""), "/");
    }
}
