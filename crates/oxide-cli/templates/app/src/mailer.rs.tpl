/// A mailer that records what it sends. Replace the body of `send` with a real transport
/// (SMTP, an email API) when you need one.
#[derive(Clone, Debug, Default, Service)]
pub struct AppMailer {
    sent: Arc<Mutex<Vec<String>>>,
}

impl AppMailer {
    /// Sends the `mailable`, e.g. `"post_created"`.
    pub fn send(&self, mailable: &str) {
        // The lock is released at the end of this statement, never held across an `.await`.
        if let Ok(mut sent) = self.sent.lock() {
            sent.push(mailable.to_owned());
        }
    }

    /// Whether `mailable` has been sent, for tests.
    pub fn was_sent(&self, mailable: &str) -> bool {
        self.sent
            .lock()
            .is_ok_and(|sent| sent.iter().any(|sent| sent == mailable))
    }
}
