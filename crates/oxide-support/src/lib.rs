//! # oxide-support
//!
//! The helper vocabulary that makes Laravel code read well, as idiomatic Rust: extension traits
//! over iterators (Collections), string, array and number helpers, relative dates, a pipeline,
//! benchmarks, lotteries and `retry`.
//!
//! Where `std` or `Iterator` already has the equivalent, there is no wrapper: see the mapping
//! table in the docs ("Support helpers").
//!
//! ```
//! use oxide_support::prelude::*;
//!
//! let users = vec![("ann", 31), ("bob", 25), ("cy", 31)];
//! let by_age = users.iter().group_by(|(_, age)| *age);
//! assert_eq!(by_age[&31].len(), 2);
//! assert_eq!(Str::slug("Hello, Oxide World!"), "hello-oxide-world");
//! assert_eq!(Number::file_size(1_536, 1), "1.5 KB");
//! ```

mod arr;
mod collection;
mod date;
mod helpers;
mod number;
mod random;
mod str;
mod tap;

pub use arr::Arr;
pub use collection::{Chunks, CollectionExt, UniqueBy};
pub use date::{diff_for_humans, diff_for_humans_from, now, today};
pub use helpers::{Benchmark, Lottery, Pipeline, retry};
pub use number::Number;
pub use str::{Str, Stringable};
pub use tap::Tap;

/// `use oxide_support::prelude::*;`
pub mod prelude {
    pub use crate::retry;
    pub use crate::{
        Arr, Benchmark, CollectionExt, Lottery, Number, Pipeline, Str, Stringable, Tap,
    };
}
