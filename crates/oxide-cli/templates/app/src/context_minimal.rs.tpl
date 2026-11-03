//! The service container: everything handlers can ask for by type.

use std::sync::{Arc, Mutex};

use oxide::prelude::*;

/// The application state. Every field is a service: a handler that takes `mailer: AppMailer`
/// receives it from here.
#[derive(Clone, Debug, Default, ServiceContainer)]
pub struct AppContext {
    pub mailer: AppMailer,
}

{{mailer}}
