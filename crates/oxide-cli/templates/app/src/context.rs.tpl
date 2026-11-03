//! The service container: everything handlers can ask for by type.

use std::sync::{Arc, Mutex};

use oxide::prelude::*;

/// The application state. Every field is a service: a handler that takes `db: DatabasePool`,
/// `mailer: AppMailer` or `Config<AppConfig>` receives it from here.
#[derive(Clone, Debug, ServiceContainer)]
pub struct AppContext {
    pub db: DatabasePool,
    pub mailer: AppMailer,
    pub config: Configuration,
}

impl AppContext {
    pub fn new(db: DatabasePool, config: Configuration) -> Self {
        Self {
            db,
            mailer: AppMailer::default(),
            config,
        }
    }
}

{{mailer}}
