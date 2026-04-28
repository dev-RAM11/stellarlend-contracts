// Common imports for WASM compilation
pub use core::clone::Clone;
pub use core::cmp::{Eq, Ord, PartialEq, PartialOrd};
pub use core::convert::{From, Into, TryFrom, TryInto};
pub use core::default::Default;
pub use core::fmt::Debug;
pub use core::iter::{ExactSizeIterator, Iterator};
pub use core::marker::Copy;
pub use core::ops::Drop;
pub use core::option::Option::{self, None, Some};
pub use core::result::Result::{self, Err, Ok};

// Re-export derive macro for struct definitions
pub use core::prelude::rust_2024::derive;

// Re-export panic macro
pub use core::panic;
