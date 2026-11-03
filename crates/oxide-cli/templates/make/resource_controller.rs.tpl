use oxide::prelude::*;

/// Route all five actions with `resource("/{{resource}}", {{controller}});` in
/// `src/routes/api.rs`. Each action answers 501 until you implement it: take `db: DatabasePool`,
/// `Bind<Model>` and `Validated<Request>` arguments as you need them.
#[derive(Debug)]
pub struct {{controller}};

impl {{controller}} {
    /// `GET /{{resource}}`
    pub async fn index() -> StatusCode {
        StatusCode::NOT_IMPLEMENTED
    }

    /// `POST /{{resource}}`
    pub async fn store() -> StatusCode {
        StatusCode::NOT_IMPLEMENTED
    }

    /// `GET /{{resource}}/{id}`
    pub async fn show() -> StatusCode {
        StatusCode::NOT_IMPLEMENTED
    }

    /// `PUT /{{resource}}/{id}`
    pub async fn update() -> StatusCode {
        StatusCode::NOT_IMPLEMENTED
    }

    /// `DELETE /{{resource}}/{id}`
    pub async fn destroy() -> StatusCode {
        StatusCode::NOT_IMPLEMENTED
    }
}
