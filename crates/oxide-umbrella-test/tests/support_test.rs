//! The helpers are reachable from `oxide::prelude` alone.

use oxide::prelude::*;

#[test]
fn the_prelude_exposes_the_support_helpers() {
    let by_len = ["a", "bb", "cc"].into_iter().group_by(|s| s.len());
    assert_eq!(by_len[&2].len(), 2);
    assert_eq!(Str::of("Hello Oxide").slug().to_string(), "hello-oxide");
    assert_eq!(Number::ordinal(3), "3rd");
    assert_eq!(5.pipe(|n| n + 1), 6);
    assert_eq!(Lottery::odds(1, 1).choose(|| "win", || "lose"), "win");
}
