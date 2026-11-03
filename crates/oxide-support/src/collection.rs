//! Laravel Collection helpers as an extension trait over any iterator.
//!
//! Every method works on `Vec`, slices (`.iter()`), maps and lazy iterators alike, so the same
//! API serves Laravel's `Collection` and `LazyCollection`. The adapters that return iterators
//! ([`chunk`](CollectionExt::chunk), [`unique_by`](CollectionExt::unique_by)) are lazy: they pull
//! from the source only as you consume them, which suits streaming large result sets.

use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::hash::Hash;
use std::iter::FusedIterator;

/// Extension methods for iterators. Import with `use oxide_support::CollectionExt;`.
///
/// Not wrapped, because `Iterator` already has them: `map`/`pluck` (`map`), `filter`/`reject`,
/// `partition`, `min_by_key`/`max_by_key`, `flat_map`, `take`/`skip`, `find` (`first_where`),
/// `any`/`all` (`contains`/`every`), `count`, `fold` (`reduce`), `zip`, `rev`, `sum`.
pub trait CollectionExt: IntoIterator + Sized {
    /// Indexes the items by a key. A later item replaces an earlier one with the same key.
    fn key_by<K: Hash + Eq>(self, mut key: impl FnMut(&Self::Item) -> K) -> HashMap<K, Self::Item> {
        self.into_iter().map(|item| (key(&item), item)).collect()
    }

    /// Groups the items by a key, keeping their order inside each group. The groups themselves
    /// are unordered; use [`group_by_ordered`](Self::group_by_ordered) to iterate them in
    /// first-seen order.
    fn group_by<K: Hash + Eq>(
        self,
        mut key: impl FnMut(&Self::Item) -> K,
    ) -> HashMap<K, Vec<Self::Item>> {
        let mut groups: HashMap<K, Vec<Self::Item>> = HashMap::new();
        for item in self {
            groups.entry(key(&item)).or_default().push(item);
        }
        groups
    }

    /// Groups the items by a key like [`group_by`](Self::group_by), but as a list in the order
    /// each key was first seen, like Laravel's `groupBy`.
    fn group_by_ordered<K: Hash + Eq + Clone>(
        self,
        mut key: impl FnMut(&Self::Item) -> K,
    ) -> Vec<(K, Vec<Self::Item>)> {
        let mut index: HashMap<K, usize> = HashMap::new();
        let mut groups: Vec<(K, Vec<Self::Item>)> = Vec::new();
        for item in self {
            let key = key(&item);
            if let Some(position) = index.get(&key) {
                groups[*position].1.push(item);
            } else {
                index.insert(key.clone(), groups.len());
                groups.push((key, vec![item]));
            }
        }
        groups
    }

    /// Counts the items per key.
    fn count_by<K: Hash + Eq>(self, mut key: impl FnMut(&Self::Item) -> K) -> HashMap<K, usize> {
        let mut counts: HashMap<K, usize> = HashMap::new();
        for item in self {
            *counts.entry(key(&item)).or_default() += 1;
        }
        counts
    }

    /// Builds a map from each item's `(key, value)` pair, like Laravel's `mapWithKeys`.
    fn map_with_keys<K: Hash + Eq, V>(
        self,
        pair: impl FnMut(Self::Item) -> (K, V),
    ) -> HashMap<K, V> {
        self.into_iter().map(pair).collect()
    }

    /// Sums a numeric projection of the items.
    fn sum_by<N: std::iter::Sum<N>>(self, project: impl FnMut(Self::Item) -> N) -> N {
        self.into_iter().map(project).sum()
    }

    /// The average of a projection, or `None` for an empty collection.
    fn avg_by(self, mut project: impl FnMut(Self::Item) -> f64) -> Option<f64> {
        let (total, count) = self.into_iter().fold((0.0, 0_u64), |(total, count), item| {
            (total + project(item), count + 1)
        });
        #[allow(clippy::cast_precision_loss)] // an average of billions of items needs no more
        (count > 0).then(|| total / count as f64)
    }

    /// Sorts ascending by a key (stable).
    fn sorted_by<K: Ord>(self, key: impl FnMut(&Self::Item) -> K) -> Vec<Self::Item> {
        let mut items: Vec<Self::Item> = self.into_iter().collect();
        items.sort_by_cached_key(key);
        items
    }

    /// Sorts descending by a key (stable), like Laravel's `sortByDesc`.
    fn sorted_by_desc<K: Ord>(self, mut key: impl FnMut(&Self::Item) -> K) -> Vec<Self::Item> {
        let mut items: Vec<Self::Item> = self.into_iter().collect();
        items.sort_by_cached_key(|item| std::cmp::Reverse(key(item)));
        items
    }

    /// Keeps the first item for each key, lazily and in order.
    fn unique_by<K: Hash + Eq, F: FnMut(&Self::Item) -> K>(
        self,
        key: F,
    ) -> UniqueBy<Self::IntoIter, K, F> {
        UniqueBy {
            iter: self.into_iter(),
            seen: HashSet::new(),
            key,
        }
    }

    /// Splits into chunks of `size`, lazily. A `size` of zero yields no chunks.
    fn chunk(self, size: usize) -> Chunks<Self::IntoIter> {
        Chunks {
            iter: self.into_iter(),
            size,
        }
    }

    /// Items whose projection equals `value`, like `where('role', 'admin')`.
    fn where_eq<V: PartialEq>(
        self,
        mut project: impl FnMut(&Self::Item) -> V,
        value: &V,
    ) -> Vec<Self::Item> {
        self.into_iter()
            .filter(|item| project(item) == *value)
            .collect()
    }

    /// Joins the items with `separator`, like `implode`.
    fn implode(self, separator: &str) -> String
    where
        Self::Item: Display,
    {
        use std::fmt::Write;

        let mut out = String::new();
        for (index, item) in self.into_iter().enumerate() {
            if index > 0 {
                out.push_str(separator);
            }
            let _ = write!(out, "{item}");
        }
        out
    }
}

impl<I: IntoIterator> CollectionExt for I {}

/// Lazy adapter returned by [`CollectionExt::unique_by`].
#[derive(Debug)]
pub struct UniqueBy<I, K, F> {
    iter: I,
    seen: HashSet<K>,
    key: F,
}

impl<I, K, F> Iterator for UniqueBy<I, K, F>
where
    I: Iterator,
    K: Hash + Eq,
    F: FnMut(&I::Item) -> K,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<I::Item> {
        loop {
            let item = self.iter.next()?;
            if self.seen.insert((self.key)(&item)) {
                return Some(item);
            }
        }
    }
}

/// Lazy adapter returned by [`CollectionExt::chunk`].
#[derive(Debug)]
pub struct Chunks<I> {
    iter: I,
    size: usize,
}

impl<I: Iterator> Iterator for Chunks<I> {
    type Item = Vec<I::Item>;

    fn next(&mut self) -> Option<Vec<I::Item>> {
        if self.size == 0 {
            return None;
        }
        let chunk: Vec<I::Item> = self.iter.by_ref().take(self.size).collect();
        (!chunk.is_empty()).then_some(chunk)
    }
}

impl<I: Iterator> FusedIterator for Chunks<I> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct User {
        name: &'static str,
        role: &'static str,
        age: u32,
    }

    fn users() -> Vec<User> {
        vec![
            User {
                name: "ann",
                role: "admin",
                age: 31,
            },
            User {
                name: "bob",
                role: "editor",
                age: 25,
            },
            User {
                name: "cy",
                role: "admin",
                age: 40,
            },
        ]
    }

    #[test]
    fn keys_groups_and_counts() {
        let by_name = users().key_by(|u| u.name);
        assert_eq!(by_name["bob"].age, 25);
        let ordered = users().group_by_ordered(|u| u.role);
        assert_eq!(
            ordered.iter().map(|(role, _)| *role).collect::<Vec<_>>(),
            ["admin", "editor"]
        );
        assert_eq!(ordered[0].1.len(), 2);
        let groups = users().group_by(|u| u.role);
        assert_eq!(
            groups["admin"].iter().map(|u| u.name).collect::<Vec<_>>(),
            ["ann", "cy"]
        );
        let counts = users().count_by(|u| u.role);
        assert_eq!((counts["admin"], counts["editor"]), (2, 1));
        let ages = users().map_with_keys(|u| (u.name, u.age));
        assert_eq!(ages["cy"], 40);
    }

    #[test]
    fn aggregates_and_sorting() {
        assert_eq!(users().sum_by(|u| u.age), 96);
        assert_eq!(users().avg_by(|u| f64::from(u.age)), Some(32.0));
        assert_eq!(Vec::<User>::new().avg_by(|u| f64::from(u.age)), None);
        let oldest_first: Vec<_> = users()
            .sorted_by_desc(|u| u.age)
            .into_iter()
            .map(|u| u.name)
            .collect();
        assert_eq!(oldest_first, ["cy", "ann", "bob"]);
        let youngest_first: Vec<_> = users()
            .sorted_by(|u| u.age)
            .into_iter()
            .map(|u| u.name)
            .collect();
        assert_eq!(youngest_first, ["bob", "ann", "cy"]);
    }

    #[test]
    fn filtering_joining_and_uniqueness() {
        assert_eq!(users().where_eq(|u| u.role, &"admin").len(), 2);
        assert_eq!(
            users().into_iter().map(|u| u.name).implode(", "),
            "ann, bob, cy"
        );
        let unique: Vec<_> = users().unique_by(|u| u.role).map(|u| u.name).collect();
        assert_eq!(unique, ["ann", "bob"]);
    }

    #[test]
    fn chunks_are_lazy_and_handle_edges() {
        let chunks: Vec<_> = (1..=7).chunk(3).collect();
        assert_eq!(chunks, [vec![1, 2, 3], vec![4, 5, 6], vec![7]]);
        assert_eq!((1..=3).chunk(0).count(), 0);
        assert_eq!(Vec::<u8>::new().chunk(2).count(), 0);

        // Only what is consumed is pulled from an endless source.
        let first: Vec<_> = (1..).chunk(2).take(2).collect();
        assert_eq!(first, [vec![1, 2], vec![3, 4]]);
        let unique_first: Vec<_> = (0..).unique_by(|n| n % 3).take(3).collect();
        assert_eq!(unique_first, [0, 1, 2]);
    }
}
