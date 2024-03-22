#![cfg_attr(feature = "doc-cfg", feature(doc_cfg))]

mod allocator;
pub mod stack;
mod stub_ctx;
mod task;

#[cfg(feature = "tree")]
#[cfg_attr(feature = "doc-cfg", doc(cfg(feature = "tree")))]
pub mod tree;

#[cfg(feature = "tree")]
#[doc(inline)]
#[cfg_attr(feature = "doc-cfg", doc(cfg(feature = "tree")))]
pub use tree::{Stk as TreeStk, TreeStack};

#[doc(inline)]
pub use stack::{Stack, Stk};

#[cfg(test)]
mod test;
