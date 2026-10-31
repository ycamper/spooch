//! `choices!`: an enum of named options that a style picks by index, a
//! button cycles through, and saved files name by key.

/// Declares the enum with each variant's display name and save key, and
/// gives it `ALL` (in declaration order), `name`, `key`, `from_key`,
/// `nth` and `index`. Mark the default variant `#[default]`.
macro_rules! choices {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident => ($label:literal, $key:literal), )+
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
        pub enum $name {
            $( $(#[$vmeta])* $variant, )+
        }

        impl $name {
            /// Every option, in the order its button steps through them.
            pub const ALL: [$name; [$($key),+].len()] = [$($name::$variant),+];

            /// What the tooltip calls it.
            pub fn name(self) -> &'static str {
                match self {
                    $( $name::$variant => $label, )+
                }
            }

            /// What saved files call it.
            pub fn key(self) -> &'static str {
                match self {
                    $( $name::$variant => $key, )+
                }
            }

            pub fn from_key(key: &str) -> Option<$name> {
                $name::ALL.into_iter().find(|c| c.key() == key)
            }

            /// Option `i` of `ALL`, wrapping round, as styles store it.
            pub fn nth(i: u8) -> $name {
                $name::ALL[usize::from(i) % $name::ALL.len()]
            }

            /// Where it is in `ALL`.
            pub fn index(self) -> u8 {
                self as u8
            }
        }
    };
}

pub(crate) use choices;
