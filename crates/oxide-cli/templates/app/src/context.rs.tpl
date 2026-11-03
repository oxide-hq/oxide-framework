//! The service container: everything handlers can ask for by type.

use std::sync::{Arc, Mutex};

use oxide::prelude::*;

/// The application state. Every field is a service: a handler that takes `db: DatabasePool` or
/// `mailer: AppMailer` receives it from here.
#[derive(Clone, Debug, ServiceContainer)]
pub struct AppContext {
    pub db: DatabasePool,
    pub mailer: AppMailer,
}

impl AppContext {
    pub fn new(db: DatabasePool) -> Self {
        Self {
            db,
            mailer: AppMailer::default(),
        }
    }
}

{{mailer}}
