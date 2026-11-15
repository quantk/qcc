//! Opaque identifiers. Each family has exactly one owner that mints it.

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, specta::Type)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }
    };
}

id!(
    /// Minted by the daemon. Opaque to providers, echoed verbatim.
    ThreadId
);
id!(
    /// Minted by the delta assembler, never by a provider or a bridge.
    TurnId
);
id!(
    /// Minted by the delta assembler, never by a provider or a bridge.
    ItemId
);
id!(
    /// Minted by the delta assembler for every `interaction.request`.
    InteractionId
);
id!(
    /// Declared by the plugin that contributes the provider.
    ProviderId
);
id!(
    /// Minted by the daemon when the user creates a project.
    ProjectId
);
