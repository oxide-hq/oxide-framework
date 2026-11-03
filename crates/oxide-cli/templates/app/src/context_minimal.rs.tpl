//! The service container: everything handlers can ask for by type.

use std::sync::{Arc, Mutex};

use oxide::prelude::*;

/// The application state. Every field is a service: a handler that takes `mailer: AppMailer`
/// or `Config<AppConfig>` receives it from here.
#[derive(Clone, Debug, ServiceContainer)]
pub struct AppContext {
    pub mailer: AppMailer,
    pub config: Configuration,
}

impl AppContext {
    pub fn new(config: Configuration) -> Self {
        Self {
            mailer: AppMailer::default(),
            config,
        }
    }
}

{{mailer}}
