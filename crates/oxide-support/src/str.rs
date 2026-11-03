//! String helpers: [`Str`] functions and the fluent [`Stringable`].

use std::fmt;
use std::ops::Deref;

use heck::{ToKebabCase, ToLowerCamelCase, ToSnakeCase, ToUpperCamelCase};

use crate::random;

const ALPHANUMERIC: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

const UNCOUNTABLE: &[&str] = &[
    "sheep",
    "fish",
    "series",
    "species",
    "news",
    "equipment",
    "information",
    "money",
    "rice",
    "deer",
];
const IRREGULAR: &[(&str, &str)] = &[
    ("person", "people"),
    ("man", "men"),
    ("woman", "women"),
    ("child", "children"),
    ("tooth", "teeth"),
    ("foot", "feet"),
    ("mouse", "mice"),
    ("goose", "geese"),
    ("ox", "oxen"),
];

/// Laravel's `Str` helpers as associated functions.
#[derive(Debug, Clone, Copy)]
pub struct Str;

impl Str {
    /// Starts a fluent chain: `Str::of("Hello").snake().limit(3, "…")`.
    pub fn of(value: impl Into<String>) -> Stringable {
        Stringable(value.into())
    }

    /// A URL slug: lowercase letters and digits joined by `-`.
    pub fn slug(value: &str) -> String {
        let mut slug = String::new();
        let mut pending_dash = false;
        for ch in value.chars() {
            if ch.is_alphanumeric() {
                if pending_dash && !slug.is_empty() {
                    slug.push('-');
                }
                pending_dash = false;
                slug.extend(ch.to_lowercase());
            } else {
                pending_dash = true;
            }
        }
        slug
    }

    /// `HelloWorld` → `hello_world`.
    pub fn snake(value: &str) -> String {
        value.to_snake_case()
    }

    /// `hello_world` → `hello-world`.
    pub fn kebab(value: &str) -> String {
        value.to_kebab_case()
    }

    /// `hello_world` → `helloWorld`.
    pub fn camel(value: &str) -> String {
        value.to_lower_camel_case()
    }

    /// `hello_world` → `HelloWorld`.
    pub fn studly(value: &str) -> String {
        value.to_upper_camel_case()
    }

    /// `hello oxide world` → `Hello Oxide World`.
    pub fn title(value: &str) -> String {
        let mut out = String::with_capacity(value.len());
        let mut start_of_word = true;
        for ch in value.chars() {
            if ch.is_alphanumeric() {
                if start_of_word {
                    out.extend(ch.to_uppercase());
                } else {
                    out.extend(ch.to_lowercase());
                }
                start_of_word = false;
            } else {
                out.push(ch);
                start_of_word = true;
            }
        }
        out
    }

    /// Upper-cases the first character.
    pub fn ucfirst(value: &str) -> String {
        let mut chars = value.chars();
        chars.next().map_or_else(String::new, |first| {
            first.to_uppercase().chain(chars).collect()
        })
    }

    /// Lower-cases the first character.
    pub fn lcfirst(value: &str) -> String {
        let mut chars = value.chars();
        chars.next().map_or_else(String::new, |first| {
            first.to_lowercase().chain(chars).collect()
        })
    }

    /// The English plural of a word: `post` → `posts`, `category` → `categories`.
    ///
    /// Handles the common rules, a table of irregulars and uncountable words. Underscored or
    /// spaced phrases pluralize their last word (`blog_post` → `blog_posts`).
    pub fn plural(value: &str) -> String {
        let (head, word) = split_last_word(value);
        let lower = word.to_lowercase();
        let plural = if UNCOUNTABLE.contains(&lower.as_str()) {
            return value.to_owned();
        } else if let Some((_, plural)) = IRREGULAR.iter().find(|(singular, _)| *singular == lower)
        {
            (*plural).to_owned()
        } else if ends_with_consonant_y(&lower) {
            format!("{}ies", &lower[..lower.len() - 1])
        } else if ["s", "x", "z", "ch", "sh"]
            .iter()
            .any(|end| lower.ends_with(end))
        {
            format!("{lower}es")
        } else {
            format!("{lower}s")
        };
        format!("{head}{}", match_case(word, &plural))
    }

    /// The English singular of a word: `posts` → `post`, `categories` → `category`.
    pub fn singular(value: &str) -> String {
        let (head, word) = split_last_word(value);
        let lower = word.to_lowercase();
        let singular = if UNCOUNTABLE.contains(&lower.as_str()) {
            return value.to_owned();
        } else if let Some((singular, _)) = IRREGULAR.iter().find(|(_, plural)| *plural == lower) {
            (*singular).to_owned()
        } else if lower.len() > 3 && lower.ends_with("ies") {
            format!("{}y", &lower[..lower.len() - 3])
        } else if ["ses", "xes", "zes", "ches", "shes"]
            .iter()
            .any(|end| lower.ends_with(end))
        {
            lower[..lower.len() - 2].to_owned()
        } else if lower.ends_with('s') && !lower.ends_with("ss") && lower.len() > 1 {
            lower[..lower.len() - 1].to_owned()
        } else {
            lower
        };
        format!("{head}{}", match_case(word, &singular))
    }

    /// At most `max` characters, with `end` appended when something was cut.
    pub fn limit(value: &str, max: usize, end: &str) -> String {
        if value.chars().count() <= max {
            return value.to_owned();
        }
        let cut: String = value.chars().take(max).collect();
        format!("{}{end}", cut.trim_end())
    }

    /// At most `max` words, with `end` appended when something was cut.
    pub fn words(value: &str, max: usize, end: &str) -> String {
        let words: Vec<&str> = value.split_whitespace().collect();
        if words.len() <= max {
            return value.to_owned();
        }
        format!("{}{end}", words[..max].join(" "))
    }

    /// Replaces characters with `ch` from `index` (negative counts from the end) for `length`
    /// characters, or to the end: `mask("secret@example.com", '*', 3, None)`.
    pub fn mask(value: &str, ch: char, index: i64, length: Option<usize>) -> String {
        let chars: Vec<char> = value.chars().collect();
        let count = i64::try_from(chars.len()).unwrap_or(i64::MAX);
        let start = if index < 0 {
            (count + index).max(0)
        } else {
            index.min(count)
        };
        let start = usize::try_from(start).unwrap_or(0);
        let end = length.map_or(chars.len(), |length| {
            start.saturating_add(length).min(chars.len())
        });
        chars
            .iter()
            .enumerate()
            .map(|(position, original)| {
                if (start..end).contains(&position) {
                    ch
                } else {
                    *original
                }
            })
            .collect()
    }

    /// Everything before the first `search`, or all of `value` when it isn't found.
    pub fn before<'a>(value: &'a str, search: &str) -> &'a str {
        value.split_once(search).map_or(value, |(before, _)| before)
    }

    /// Everything after the first `search`, or all of `value` when it isn't found.
    pub fn after<'a>(value: &'a str, search: &str) -> &'a str {
        value.split_once(search).map_or(value, |(_, after)| after)
    }

    /// Everything before the last `search`.
    pub fn before_last<'a>(value: &'a str, search: &str) -> &'a str {
        value
            .rsplit_once(search)
            .map_or(value, |(before, _)| before)
    }

    /// Everything after the last `search`.
    pub fn after_last<'a>(value: &'a str, search: &str) -> &'a str {
        value.rsplit_once(search).map_or(value, |(_, after)| after)
    }

    /// The text between the first `from` and the last `to` after it.
    pub fn between<'a>(value: &'a str, from: &str, to: &str) -> &'a str {
        if from.is_empty() || to.is_empty() {
            return value;
        }
        Self::before_last(Self::after(value, from), to)
    }

    /// Ensures `value` ends with `cap`, without doubling it.
    pub fn finish(value: &str, cap: &str) -> String {
        if value.ends_with(cap) {
            value.to_owned()
        } else {
            format!("{value}{cap}")
        }
    }

    /// Ensures `value` starts with `prefix`, without doubling it.
    pub fn start(value: &str, prefix: &str) -> String {
        if value.starts_with(prefix) {
            value.to_owned()
        } else {
            format!("{prefix}{value}")
        }
    }

    /// Collapses runs of whitespace into single spaces and trims the ends.
    pub fn squish(value: &str) -> String {
        value.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// Replaces only the first occurrence.
    pub fn replace_first(value: &str, search: &str, replace: &str) -> String {
        if search.is_empty() {
            return value.to_owned();
        }
        value.replacen(search, replace, 1)
    }

    /// Replaces only the last occurrence.
    pub fn replace_last(value: &str, search: &str, replace: &str) -> String {
        match value.rfind(search).filter(|_| !search.is_empty()) {
            Some(at) => format!("{}{replace}{}", &value[..at], &value[at + search.len()..]),
            None => value.to_owned(),
        }
    }

    /// A random (v4) UUID.
    pub fn uuid() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    /// A time-ordered (v7) UUID, friendly to database indexes.
    pub fn uuid7() -> String {
        uuid::Uuid::now_v7().to_string()
    }

    /// A ULID: sortable, 26 characters.
    pub fn ulid() -> String {
        ulid::Ulid::generate().to_string()
    }

    pub fn is_uuid(value: &str) -> bool {
        uuid::Uuid::parse_str(value).is_ok()
    }

    pub fn is_ulid(value: &str) -> bool {
        ulid::Ulid::from_string(value).is_ok()
    }

    /// A cryptographically random alphanumeric string of `length` characters.
    pub fn random(length: usize) -> String {
        (0..length)
            .map(|_| {
                let index = usize::try_from(random::below(ALPHANUMERIC.len() as u64)).unwrap_or(0);
                char::from(ALPHANUMERIC[index])
            })
            .collect()
    }
}

/// Splits `blog_post` into (`blog_`, `post`) so only the last word is inflected.
fn split_last_word(value: &str) -> (&str, &str) {
    match value.rfind(['_', ' ', '-']) {
        Some(at) => value.split_at(at + 1),
        None => ("", value),
    }
}

fn ends_with_consonant_y(word: &str) -> bool {
    let mut rev = word.chars().rev();
    rev.next() == Some('y') && rev.next().is_some_and(|c| !"aeiou".contains(c))
}

/// Gives `inflected` the capitalisation of `original` (`Person` → `People`, `PERSON` → `PEOPLE`).
fn match_case(original: &str, inflected: &str) -> String {
    if original.chars().count() > 1 && original.chars().all(|c| !c.is_lowercase()) {
        inflected.to_uppercase()
    } else if original.chars().next().is_some_and(char::is_uppercase) {
        Str::ucfirst(inflected)
    } else {
        inflected.to_owned()
    }
}

/// A string with chainable helpers, like Laravel's `Stringable`.
///
/// ```
/// use oxide_support::Str;
/// assert_eq!(Str::of("Hello Oxide World").slug().limit(11, "…").to_string(), "hello-oxide…");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Stringable(String);

macro_rules! chain {
    ($($(#[$doc:meta])* $name:ident => $function:path;)*) => {$(
        $(#[$doc])*
        #[must_use]
        pub fn $name(self) -> Self {
            Self($function(&self.0))
        }
    )*};
}

impl Stringable {
    chain! {
        slug => Str::slug;
        snake => Str::snake;
        kebab => Str::kebab;
        camel => Str::camel;
        studly => Str::studly;
        title => Str::title;
        ucfirst => Str::ucfirst;
        lcfirst => Str::lcfirst;
        plural => Str::plural;
        singular => Str::singular;
        squish => Str::squish;
    }

    #[must_use]
    pub fn limit(self, max: usize, end: &str) -> Self {
        Self(Str::limit(&self.0, max, end))
    }

    #[must_use]
    pub fn words(self, max: usize, end: &str) -> Self {
        Self(Str::words(&self.0, max, end))
    }

    #[must_use]
    pub fn mask(self, ch: char, index: i64, length: Option<usize>) -> Self {
        Self(Str::mask(&self.0, ch, index, length))
    }

    #[must_use]
    pub fn before(self, search: &str) -> Self {
        Self(Str::before(&self.0, search).to_owned())
    }

    #[must_use]
    pub fn after(self, search: &str) -> Self {
        Self(Str::after(&self.0, search).to_owned())
    }

    #[must_use]
    pub fn between(self, from: &str, to: &str) -> Self {
        Self(Str::between(&self.0, from, to).to_owned())
    }

    #[must_use]
    pub fn finish(self, cap: &str) -> Self {
        Self(Str::finish(&self.0, cap))
    }

    #[must_use]
    pub fn start(self, prefix: &str) -> Self {
        Self(Str::start(&self.0, prefix))
    }

    #[must_use]
    pub fn append(mut self, suffix: &str) -> Self {
        self.0.push_str(suffix);
        self
    }

    #[must_use]
    pub fn prepend(self, prefix: &str) -> Self {
        Self(format!("{prefix}{}", self.0))
    }

    #[must_use]
    pub fn upper(self) -> Self {
        Self(self.0.to_uppercase())
    }

    #[must_use]
    pub fn lower(self) -> Self {
        Self(self.0.to_lowercase())
    }

    #[must_use]
    pub fn trim(self) -> Self {
        Self(self.0.trim().to_owned())
    }

    #[must_use]
    pub fn replace(self, search: &str, replace: &str) -> Self {
        Self(self.0.replace(search, replace))
    }

    /// The underlying `String`.
    pub fn into_string(self) -> String {
        self.0
    }
}

impl Deref for Stringable {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Stringable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for Stringable {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for Stringable {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<Stringable> for String {
    fn from(value: Stringable) -> Self {
        value.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Tap;

    #[test]
    fn cases() {
        assert_eq!(Str::slug("  Hello, Oxide World!  "), "hello-oxide-world");
        assert_eq!(Str::slug("Crème brûlée & más"), "crème-brûlée-más");
        assert_eq!(Str::snake("HelloOxideWorld"), "hello_oxide_world");
        assert_eq!(Str::kebab("hello_oxide"), "hello-oxide");
        assert_eq!(Str::camel("hello_oxide"), "helloOxide");
        assert_eq!(Str::studly("hello_oxide"), "HelloOxide");
        assert_eq!(Str::title("hello oxide-world"), "Hello Oxide-World");
        assert_eq!(Str::ucfirst("élan"), "Élan");
        assert_eq!(Str::lcfirst("Oxide"), "oxide");
        assert_eq!(Str::ucfirst(""), "");
    }

    #[test]
    fn inflection() {
        for (singular, plural) in [
            ("post", "posts"),
            ("category", "categories"),
            ("day", "days"),
            ("box", "boxes"),
            ("church", "churches"),
            ("bus", "buses"),
            ("person", "people"),
            ("Person", "People"),
            ("blog_post", "blog_posts"),
            ("blog_category", "blog_categories"),
            ("sheep", "sheep"),
        ] {
            assert_eq!(Str::plural(singular), plural, "plural of {singular}");
            assert_eq!(Str::singular(plural), singular, "singular of {plural}");
        }
        assert_eq!(Str::singular("class"), "class");
        assert_eq!(Str::plural("PERSON"), "PEOPLE");
    }

    #[test]
    fn trimming_and_slicing() {
        assert_eq!(Str::limit("The quick brown fox", 9, "…"), "The quick…");
        assert_eq!(Str::limit("short", 9, "…"), "short");
        assert_eq!(Str::limit("héllo wörld", 5, "..."), "héllo...");
        assert_eq!(Str::words("one two three four", 2, " >>"), "one two >>");
        assert_eq!(Str::words("one two", 5, ">>"), "one two");
        assert_eq!(Str::before("a@b@c", "@"), "a");
        assert_eq!(Str::after("a@b@c", "@"), "b@c");
        assert_eq!(Str::before_last("a@b@c", "@"), "a@b");
        assert_eq!(Str::after_last("a@b@c", "@"), "c");
        assert_eq!(Str::between("[a [b] c]", "[", "]"), "a [b] c");
        assert_eq!(Str::between("none", "[", "]"), "none");
        assert_eq!(Str::finish("path", "/"), "path/");
        assert_eq!(Str::finish("path/", "/"), "path/");
        assert_eq!(Str::start("path", "/"), "/path");
        assert_eq!(Str::squish("  a \n b\t c  "), "a b c");
        assert_eq!(Str::replace_first("a-b-c", "-", "+"), "a+b-c");
        assert_eq!(Str::replace_last("a-b-c", "-", "+"), "a-b+c");
        assert_eq!(Str::replace_last("abc", "x", "+"), "abc");
    }

    #[test]
    fn masking() {
        assert_eq!(
            Str::mask("secret@example.com", '*', 3, None),
            "sec***************"
        );
        assert_eq!(Str::mask("secret", '*', -2, None), "secr**");
        assert_eq!(Str::mask("secret", '*', 1, Some(2)), "s**ret");
        assert_eq!(Str::mask("secret", '*', 99, None), "secret");
        assert_eq!(Str::mask("secret", '*', -99, Some(1)), "*ecret");
    }

    #[test]
    fn identifiers_and_randomness() {
        assert!(Str::is_uuid(&Str::uuid()));
        assert!(Str::is_uuid(&Str::uuid7()));
        assert!(!Str::is_uuid("nope"));
        assert_eq!(Str::ulid().len(), 26);
        assert!(Str::is_ulid(&Str::ulid()));
        assert!(!Str::is_ulid("nope"));
        let token = Str::random(40);
        assert_eq!(token.len(), 40);
        assert!(token.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(token, Str::random(40));
        assert_eq!(Str::random(0), "");
    }

    #[test]
    fn stringable_chains() {
        let title = Str::of("Hello Oxide World").slug().limit(11, "…");
        assert_eq!(title.to_string(), "hello-oxide…");
        assert_eq!(title.len(), "hello-oxide…".len());
        let text = Str::of("post_category")
            .studly()
            .append("!")
            .prepend(">")
            .upper()
            .replace("!", "?")
            .when(true, |s| s.finish("."));
        assert_eq!(String::from(text), ">POSTCATEGORY?.");
        assert_eq!(Str::of("a.b").before(".").into_string(), "a");
        assert_eq!(Str::of("a.b").after(".").to_string(), "b");
        assert_eq!(Str::of("[x]").between("[", "]").to_string(), "x");
        assert_eq!(Str::of("  pad ").trim().lower().to_string(), "pad");
        assert_eq!(Str::of("abc").mask('*', 1, None).to_string(), "a**");
        assert_eq!(Str::of("a b c").words(2, "").to_string(), "a b");
        assert_eq!(
            Str::of("box").plural().singular().camel().to_string(),
            "box"
        );
        assert_eq!(Str::of("x").start("/").to_string(), "/x");
        assert_eq!(
            Stringable::from("snake case")
                .kebab()
                .ucfirst()
                .lcfirst()
                .title()
                .snake()
                .squish()
                .to_string(),
            "snake_case"
        );
    }
}
