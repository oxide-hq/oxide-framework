use oxide::axum::extract::Request;
use oxide::foundation::MiddlewareFuture;
use oxide::prelude::*;

/// Register it in `build_app` with `.middleware({{middleware}})` to run it for every request, or
/// `.alias("{{alias}}", {{middleware}})` and use it on routes:
/// `middleware(route_middleware(&["{{alias}}"])) { .. }`.
#[derive(Debug, Clone, Copy)]
pub struct {{middleware}};

impl Middleware for {{middleware}} {
    fn handle<'a>(
        &'a self,
        request: Request,
        next: MwNext,
        _params: &'a [String],
    ) -> MiddlewareFuture<'a> {
        Box::pin(async move {
            // Inspect or change the request here. Return a response instead of calling `next` to
            // stop it, for example `OxideHttpError::Forbidden(String::new()).into_response()`.
            // `_params` holds what follows the alias's colon: `"{{alias}}:a,b"` gives `["a", "b"]`.
            // After `.run(..)` you hold the response and can change it before it goes out.
            next.run(request).await
        })
    }
}
