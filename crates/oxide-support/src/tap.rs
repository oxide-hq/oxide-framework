//! `pipe`, `tap` and conditional chaining for any value.

/// Fluent helpers available on every sized value.
pub trait Tap: Sized {
    /// Passes `self` to `f` and returns the result, like Laravel's `pipe`.
    fn pipe<R>(self, f: impl FnOnce(Self) -> R) -> R {
        f(self)
    }

    /// Runs `f` on a reference for its side effect and returns `self`.
    #[must_use]
    fn tap(self, f: impl FnOnce(&Self)) -> Self {
        f(&self);
        self
    }

    /// Applies `f` only when `condition` is true.
    #[must_use]
    fn when(self, condition: bool, f: impl FnOnce(Self) -> Self) -> Self {
        if condition { f(self) } else { self }
    }

    /// Applies `f` only when `condition` is false.
    #[must_use]
    fn unless(self, condition: bool, f: impl FnOnce(Self) -> Self) -> Self {
        self.when(!condition, f)
    }
}

impl<T> Tap for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_tap_and_when() {
        let mut seen = 0;
        let value = 2
            .tap(|n| seen = *n)
            .when(true, |n| n * 10)
            .unless(true, |n| n + 1000)
            .pipe(|n| n + 1);
        assert_eq!((value, seen), (21, 2));
    }
}
