//! Shared validation and macro support for string-backed newtypes.
//!
//! The generated newtypes centralize validation for string identifiers
//! passed through public transport and worker APIs. Centralizing the
//! validation closes a header-injection seam that would otherwise require
//! every caller to re-validate string inputs before forwarding them to
//! provider HTTP headers, queues, or logs.

/// Maximum byte length of a string-newtype value.
///
/// 1 KiB is comfortably above any real tenant / trace / idempotency
/// identifier and well below any limit a queue or log line will impose.
pub const STRING_NEWTYPE_MAX_BYTES: usize = 1024;

/// Reasons a string value cannot be promoted into a validated string newtype
/// defined via the [`crate::string_newtype!`] macro.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum StringNewtypeError {
    #[error("string newtype value cannot be empty")]
    Empty,
    #[error("string newtype value contains a NUL byte")]
    Nul,
    #[error("string newtype value contains a CR or LF byte")]
    Newline,
    #[error("string newtype value contains a non-tab control character")]
    Control,
    #[error("string newtype value exceeds {max} bytes (got {len})")]
    #[non_exhaustive]
    TooLong { len: usize, max: usize },
    /// The value contains a Unicode-tag codepoint (U+E0000..U+E007F).
    /// These are invisible to humans but readable by some downstream
    /// tooling, a known "ASCII smuggling" / log-injection vector.
    /// Rejected to keep the type-level "no header injection, ever"
    /// contract honest beyond the ASCII-control byte set.
    ///
    /// # Scope
    ///
    /// The Unicode-tag rejection (U+E0000..U+E007F) is scoped to the
    /// ASCII-smuggling family only. Other zero-width and
    /// visually-confusable codepoints (U+200B..U+200D zero-width
    /// spaces, U+2060 word-joiner, U+FEFF BOM, the U+FE00..U+FE0F
    /// variation selectors, U+E0100..U+E01EF supplementary variation
    /// selectors) are out of scope by design: legitimate text in many
    /// scripts (Korean, Hebrew, several CJK renderings) uses them and
    /// rejecting them would break valid identifier values. Callers
    /// that need a stricter character class should layer their own
    /// normalization or rejection on top.
    #[error("string newtype value contains a Unicode-tag codepoint (U+E0000..U+E007F)")]
    UnicodeTag,
}

/// Validates an input for a string newtype: rejects empty, NUL, CR/LF,
/// non-tab control chars, Unicode-tag codepoints
/// (U+E0000..U+E007F, invisible-to-humans "ASCII smuggling" vector),
/// and values longer than [`STRING_NEWTYPE_MAX_BYTES`]. See
/// [`StringNewtypeError::UnicodeTag`] for the scope of the Unicode-tag
/// rejection (which other zero-width codepoints are deliberately *not*
/// rejected).
///
/// `pub` so the [`string_newtype!`] macro can call it from outside this
/// crate; not part of the curated rustdoc surface (downstream callers
/// construct values via the typed `new` constructor on each newtype).
#[doc(hidden)]
pub fn __validate_string_newtype(value: &str) -> Result<(), StringNewtypeError> {
    if value.is_empty() {
        return Err(StringNewtypeError::Empty);
    }
    if value.len() > STRING_NEWTYPE_MAX_BYTES {
        return Err(StringNewtypeError::TooLong {
            len: value.len(),
            max: STRING_NEWTYPE_MAX_BYTES,
        });
    }
    for ch in value.chars() {
        // Unicode-tag block: U+E0000..U+E007F. These are not control
        // chars under `is_ascii_control` (they're 4-byte UTF-8
        // sequences) but downstream log/header processors render
        // them as zero-width. Rejecting closes the smuggling seam
        // the docstring claims to close.
        if ('\u{E0000}'..='\u{E007F}').contains(&ch) {
            return Err(StringNewtypeError::UnicodeTag);
        }
    }
    for byte in value.bytes() {
        if byte == 0 {
            return Err(StringNewtypeError::Nul);
        }
        if byte == b'\r' || byte == b'\n' {
            return Err(StringNewtypeError::Newline);
        }
        if byte != b'\t' && byte.is_ascii_control() {
            return Err(StringNewtypeError::Control);
        }
    }
    Ok(())
}

/// Generates a `String`-backed validated newtype with the standard derives
/// and conversion impls.
///
/// `#[macro_export]` so that downstream crates can define their own
/// siblings (`TenantId`, `TraceId`, `TransportProfile`) without
/// re-implementing the validation surface. Generated types validate via
/// [`__validate_string_newtype`] on construction, reject empty / NUL /
/// CR-LF / non-tab control / > 1 KiB.
///
/// # Variants
///
/// Two matcher arms:
///
/// - **`string_newtype! { Name }`**, base form. Generates the safe
///   `new` constructor (validating), `from_str` / `try_from` /
///   `into_inner` / `as_str` / `Display`, plus `Clone` / `Debug` /
///   `PartialEq` / `Eq` / `Hash`. With the default-enabled `serde`
///   feature, it also generates `Serialize` / `Deserialize`; with the
///   `schemars` feature, it generates `JsonSchema`.
/// - **`string_newtype! { @unchecked Name }`**, adds an additional
///   `pub fn new_unchecked` that skips validation. Reserved for trusted
///   inputs only (internal constants in trusted code paths). The
///   `@unchecked` opt-in shape avoids inheriting a footgun on every
///   newtype the macro generates downstream.
///
/// # Serde dependency hygiene
///
/// With the `serde` feature enabled (on by default), the macro routes the
/// `Serialize` / `Deserialize` impls through `email_transport::__macro_serde`
/// (a `#[doc(hidden)]` re-export of `serde`). Downstream users do **not** need
/// a direct `serde` Cargo dep, the kernel provides the path. The Deserialize
/// impl routes through the generated newtype's `new` constructor, so a
/// malformed value (CRLF, NUL, oversized, etc.) cannot enter via serde.
#[macro_export]
macro_rules! string_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $crate::__string_newtype_struct!($(#[$meta])* $name);
        $crate::__string_newtype_impls!($name);
    };

    ($(#[$meta:meta])* @unchecked $name:ident) => {
        $crate::__string_newtype_struct!($(#[$meta])* $name);
        $crate::__string_newtype_impls!($name);
        $crate::__string_newtype_unchecked!($name);
    };
}

/// Internal helper of [`string_newtype!`], defines the struct itself.
/// Not part of the public surface.
#[doc(hidden)]
#[macro_export]
macro_rules! __string_newtype_struct {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        pub struct $name(::std::string::String);
    };
}

/// Internal helper of [`string_newtype!`], emits the standard impl set
/// (validating constructors, accessors, Display, FromStr, TryFrom, From,
/// AsRef). Not part of the public surface.
#[doc(hidden)]
#[macro_export]
macro_rules! __string_newtype_impls {
    ($name:ident) => {
        impl $name {
            /// Construct after validating the input.
            ///
            /// # Errors
            ///
            /// Returns [`$crate::StringNewtypeError`] if the value is
            /// empty, exceeds `STRING_NEWTYPE_MAX_BYTES`, or contains
            /// NUL, CR/LF, or non-tab control characters.
            pub fn new(
                value: impl ::std::convert::Into<::std::string::String>,
            ) -> ::std::result::Result<Self, $crate::StringNewtypeError> {
                let value = value.into();
                $crate::string_newtype::__validate_string_newtype(&value)?;
                ::std::result::Result::Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }

            #[must_use]
            pub fn into_inner(self) -> ::std::string::String {
                self.0
            }
        }

        impl ::std::convert::AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.0.as_str()
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl ::std::str::FromStr for $name {
            type Err = $crate::StringNewtypeError;
            fn from_str(s: &str) -> ::std::result::Result<Self, Self::Err> {
                Self::new(s.to_owned())
            }
        }

        impl ::std::convert::TryFrom<::std::string::String> for $name {
            type Error = $crate::StringNewtypeError;
            fn try_from(value: ::std::string::String) -> ::std::result::Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl ::std::convert::TryFrom<&str> for $name {
            type Error = $crate::StringNewtypeError;
            fn try_from(value: &str) -> ::std::result::Result<Self, Self::Error> {
                Self::new(value.to_owned())
            }
        }

        impl ::std::convert::From<$name> for ::std::string::String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        $crate::__string_newtype_serde_impls!($name);
        $crate::__string_newtype_schemars_impls!($name);
    };
}

/// Internal helper of [`string_newtype!`], emits serde impls when the
/// `serde` feature is enabled. This is a separate macro so the feature gate
/// is evaluated in `email-transport`, not in downstream crates that invoke
/// [`string_newtype!`]. Not part of the public surface.
#[cfg(feature = "serde")]
#[doc(hidden)]
#[macro_export]
macro_rules! __string_newtype_serde_impls {
    ($name:ident) => {
        impl $crate::__macro_serde::Serialize for $name {
            fn serialize<S>(
                &self,
                serializer: S,
            ) -> ::std::result::Result<S::Ok, S::Error>
            where
                S: $crate::__macro_serde::Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> $crate::__macro_serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> ::std::result::Result<Self, D::Error>
            where
                D: $crate::__macro_serde::Deserializer<'de>,
            {
                use $crate::__macro_serde::de::Error as _;
                let value = <::std::string::String as $crate::__macro_serde::Deserialize<'de>>::deserialize(deserializer)?;
                Self::new(value).map_err(D::Error::custom)
            }
        }
    };
}

/// Internal helper of [`string_newtype!`], intentionally no-ops when the
/// `serde` feature is disabled. Not part of the public surface.
#[cfg(not(feature = "serde"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __string_newtype_serde_impls {
    ($name:ident) => {};
}

/// Internal helper of [`string_newtype!`], emits schemars impls when the
/// `schemars` feature is enabled. Not part of the public surface.
#[cfg(feature = "schemars")]
#[doc(hidden)]
#[macro_export]
macro_rules! __string_newtype_schemars_impls {
    ($name:ident) => {
        impl $crate::__macro_schemars::JsonSchema for $name {
            fn inline_schema() -> bool {
                true
            }

            fn schema_name() -> ::std::borrow::Cow<'static, str> {
                stringify!($name).into()
            }

            fn schema_id() -> ::std::borrow::Cow<'static, str> {
                concat!(module_path!(), "::", stringify!($name)).into()
            }

            fn json_schema(
                _generator: &mut $crate::__macro_schemars::SchemaGenerator,
            ) -> $crate::__macro_schemars::Schema {
                $crate::__macro_schemars::json_schema!({
                    "type": "string",
                    "minLength": 1,
                    "maxLength": $crate::STRING_NEWTYPE_MAX_BYTES,
                })
            }
        }
    };
}

/// Internal helper of [`string_newtype!`], intentionally no-ops when the
/// `schemars` feature is disabled. Not part of the public surface.
#[cfg(not(feature = "schemars"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __string_newtype_schemars_impls {
    ($name:ident) => {};
}

/// Internal helper of [`string_newtype!`], emits the opt-in
/// `new_unchecked` constructor. Not part of the public surface;
/// reachable through the `@unchecked` matcher arm of
/// [`string_newtype!`].
#[doc(hidden)]
#[macro_export]
macro_rules! __string_newtype_unchecked {
    ($name:ident) => {
        impl $name {
            /// Construct **without** validating.
            ///
            /// Reserved for trusted inputs (e.g. internal constants in
            /// trusted code paths). Prefer [`Self::new`] or
            /// [`std::str::FromStr`] for any value that originated
            /// outside your own code.
            ///
            /// This constructor exists only because the type was
            /// declared via the `@unchecked` matcher arm of
            /// [`crate::string_newtype!`]; the default arm omits it.
            #[must_use]
            pub fn new_unchecked(value: impl ::std::convert::Into<::std::string::String>) -> Self {
                Self(value.into())
            }
        }
    };
}
