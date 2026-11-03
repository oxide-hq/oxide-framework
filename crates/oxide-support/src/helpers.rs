//! Small utilities: [`Pipeline`], [`Benchmark`], [`Lottery`] and [`retry`].
//!
//! `once` is `std::sync::OnceLock` / `LazyLock`, so there is no wrapper.

use std::future::Future;
use std::time::{Duration, Instant};

use crate::random;

type Pipe<T, R> = Box<dyn Fn(T, &dyn Fn(T) -> R) -> R + Send + Sync>;

/// Passes a value through a stack of steps, each deciding whether to call the next one, like
/// Laravel's `Pipeline`.
///
/// ```
/// use oxide_support::Pipeline;
///
/// let result = Pipeline::send(String::from(" hello "))
///     .through(|text, next| next(text.trim().to_owned()))
///     .through(|text, next| next(text.to_uppercase()))
///     .then(|text| text + "!");
/// assert_eq!(result, "HELLO!");
/// ```
pub struct Pipeline<T, R = T> {
    passable: T,
    pipes: Vec<Pipe<T, R>>,
}

impl<T, R> std::fmt::Debug for Pipeline<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pipeline")
            .field("pipes", &self.pipes.len())
            .finish_non_exhaustive()
    }
}

impl<T, R> Pipeline<T, R> {
    /// Starts a pipeline with the value to pass along.
    pub fn send(passable: T) -> Self {
        Self {
            passable,
            pipes: Vec::new(),
        }
    }

    /// Adds a step. It receives the value and `next`; return `next(value)` to continue, or
    /// anything else to short-circuit the rest.
    #[must_use]
    pub fn through(
        mut self,
        pipe: impl Fn(T, &dyn Fn(T) -> R) -> R + Send + Sync + 'static,
    ) -> Self {
        self.pipes.push(Box::new(pipe));
        self
    }

    /// Runs the steps in order, ending in `destination`.
    pub fn then(self, destination: impl Fn(T) -> R) -> R {
        fn run<T, R>(pipes: &[Pipe<T, R>], destination: &dyn Fn(T) -> R, passable: T) -> R {
            match pipes.split_first() {
                None => destination(passable),
                Some((first, rest)) => first(passable, &|value| run(rest, destination, value)),
            }
        }
        run(&self.pipes, &destination, self.passable)
    }
}

/// Times closures, like Laravel's `Benchmark`.
#[derive(Debug, Clone, Copy)]
pub struct Benchmark;

impl Benchmark {
    /// How long `f` takes.
    pub fn measure(f: impl FnOnce()) -> Duration {
        Self::value(f).1
    }

    /// The result of `f` and how long it took.
    pub fn value<T>(f: impl FnOnce() -> T) -> (T, Duration) {
        let started = Instant::now();
        let result = f();
        (result, started.elapsed())
    }

    /// The average time of `iterations` runs of `f` (zero iterations give a zero duration).
    pub fn average(iterations: u32, mut f: impl FnMut()) -> Duration {
        if iterations == 0 {
            return Duration::ZERO;
        }
        let started = Instant::now();
        for _ in 0..iterations {
            f();
        }
        started.elapsed() / iterations
    }
}

/// A chance to win, like Laravel's `Lottery`: `Lottery::odds(1, 20)` wins one time in twenty.
///
/// Typical use is sampling, such as logging one request in a hundred.
#[derive(Debug, Clone, Copy)]
pub struct Lottery {
    chances: u64,
    out_of: u64,
}

impl Lottery {
    pub fn odds(chances: u64, out_of: u64) -> Self {
        Self { chances, out_of }
    }

    /// Draws once.
    pub fn wins(self) -> bool {
        // `out_of` of zero is no lottery at all: nobody wins.
        self.out_of > 0 && self.wins_with(random::below(self.out_of))
    }

    /// The outcome for a given draw in `0..out_of`: deterministic, for tests.
    pub fn wins_with(self, draw: u64) -> bool {
        draw < self.chances
    }

    /// Runs `winner` or `loser` after a draw and returns its result.
    pub fn choose<T>(self, winner: impl FnOnce() -> T, loser: impl FnOnce() -> T) -> T {
        if self.wins() { winner() } else { loser() }
    }
}

/// Calls `f` up to `times` times (at least once), sleeping `delay` between failures, and returns
/// the first success or the last error. `f` receives the attempt number, starting at 1.
///
/// ```
/// # tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap().block_on(async {
/// use std::time::Duration;
/// use oxide_support::retry;
///
/// let value = retry(3, Duration::ZERO, |attempt| async move {
///     if attempt < 3 { Err("not yet") } else { Ok(attempt) }
/// })
/// .await;
/// assert_eq!(value, Ok(3));
/// # });
/// ```
pub async fn retry<T, E, F, Fut>(times: u32, delay: Duration, mut f: F) -> Result<T, E>
where
    F: FnMut(u32) -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let attempts = times.max(1);
    let mut attempt = 1;
    loop {
        match f(attempt).await {
            Ok(value) => return Ok(value),
            Err(error) if attempt >= attempts => return Err(error),
            Err(_) => {
                attempt += 1;
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    #[test]
    fn pipeline_runs_steps_in_order_and_can_short_circuit() {
        let order = Pipeline::<Vec<&str>>::send(vec![])
            .through(|mut v, next| {
                v.push("a");
                next(v)
            })
            .through(|mut v, next| {
                v.push("b");
                next(v)
            })
            .then(|mut v| {
                v.push("end");
                v
            });
        assert_eq!(order, ["a", "b", "end"]);

        let stopped = Pipeline::send(1)
            .through(|n, _next| n + 100)
            .through(|n, next| next(n + 1000))
            .then(|n| n);
        assert_eq!(stopped, 101);

        let untouched = Pipeline::send(7).then(|n| n * 2);
        assert_eq!(untouched, 14);
        assert!(format!("{:?}", Pipeline::<i32>::send(1)).contains("Pipeline"));
    }

    #[test]
    fn pipeline_can_change_the_result_type() {
        let length = Pipeline::<String, usize>::send("abc".into())
            .through(|text, next| next(text + "d"))
            .then(|text| text.len());
        assert_eq!(length, 4);
    }

    #[test]
    fn benchmarks_return_values_and_durations() {
        let (value, took) = Benchmark::value(|| 21 * 2);
        assert_eq!(value, 42);
        assert!(took < Duration::from_secs(5));
        let mut calls = 0;
        let _ = Benchmark::average(4, || calls += 1);
        assert_eq!(calls, 4);
        assert_eq!(Benchmark::average(0, || unreachable!()), Duration::ZERO);
        assert!(Benchmark::measure(|| {}) < Duration::from_secs(5));
    }

    #[test]
    fn lottery_is_deterministic_with_a_draw() {
        let odds = Lottery::odds(1, 4);
        assert!(odds.wins_with(0));
        assert!(!odds.wins_with(1));
        assert!(Lottery::odds(5, 5).wins());
        assert!(!Lottery::odds(0, 5).wins());
        assert!(!Lottery::odds(1, 0).wins());
        assert_eq!(Lottery::odds(5, 5).choose(|| "won", || "lost"), "won");
        assert_eq!(Lottery::odds(0, 5).choose(|| "won", || "lost"), "lost");
        let wins = (0..2000).filter(|_| Lottery::odds(1, 2).wins()).count();
        assert!((600..1400).contains(&wins), "fair coin won {wins}/2000");
    }

    #[tokio::test]
    async fn retry_stops_at_the_first_success_or_the_last_error() {
        let calls = AtomicU32::new(0);
        let ok = retry(5, Duration::ZERO, |attempt| {
            calls.fetch_add(1, Ordering::SeqCst);
            async move { if attempt == 2 { Ok(attempt) } else { Err("no") } }
        })
        .await;
        assert_eq!((ok, calls.load(Ordering::SeqCst)), (Ok(2), 2));

        let failed: Result<(), u32> = retry(3, Duration::from_millis(1), |attempt| async move {
            Err(attempt)
        })
        .await;
        assert_eq!(failed, Err(3));

        let once: Result<(), &str> = retry(0, Duration::ZERO, |_| async { Err("only once") }).await;
        assert_eq!(once, Err("only once"));
    }
}
