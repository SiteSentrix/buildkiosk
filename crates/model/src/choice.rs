//! Closed sets of values (statuses, types, priorities) with stable keys and English labels.

/// A closed set of values. The key is the stable spelling in JSON, forms and journals (never change
/// it once released); the label is what people read.
pub trait Choice: Sized + Copy + PartialEq + 'static {
    const ALL: &'static [Self];

    fn key(self) -> &'static str;

    fn label(self) -> &'static str;

    /// Matches a key or a label, ignoring case and surrounding space.
    fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        Self::ALL.iter().copied().find(|c| c.key().eq_ignore_ascii_case(s) || c.label().eq_ignore_ascii_case(s))
    }
}

/// Declares an enum implementing [`Choice`], serialized as its key:
///
/// ```
/// buildkiosk_model::choice! {
///     /// How urgent an item is.
///     pub enum Priority { Low = ("low", "Low"), High = ("high", "High") }
/// }
/// ```
#[macro_export]
macro_rules! choice {
    ($(#[$meta:meta])* $vis:vis enum $name:ident { $($(#[$vmeta:meta])* $var:ident = ($key:literal, $label:literal)),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        $vis enum $name { $($(#[$vmeta])* $var),+ }

        impl $crate::Choice for $name {
            const ALL: &'static [Self] = &[$($name::$var),+];
            fn key(self) -> &'static str {
                match self { $($name::$var => $key),+ }
            }
            fn label(self) -> &'static str {
                match self { $($name::$var => $label),+ }
            }
        }

        impl $crate::__serde::Serialize for $name {
            fn serialize<S: $crate::__serde::Serializer>(&self, s: S) -> ::std::result::Result<S::Ok, S::Error> {
                s.serialize_str($crate::Choice::key(*self))
            }
        }

        impl<'de> $crate::__serde::Deserialize<'de> for $name {
            fn deserialize<D: $crate::__serde::Deserializer<'de>>(d: D) -> ::std::result::Result<Self, D::Error> {
                let s = <::std::string::String as $crate::__serde::Deserialize>::deserialize(d)?;
                <$name as $crate::Choice>::parse(&s)
                    .ok_or_else(|| <D::Error as $crate::__serde::de::Error>::custom(format!("unknown {} `{}`", stringify!($name), s)))
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str($crate::Choice::label(*self))
            }
        }
    };
}
