#![cfg_attr(docrs, feature(doc_cfg))]

mod allocator;
pub mod stack;
mod stub_ctx;

mod defer;
#[cfg(feature = "tree")]
#[cfg_attr(docrs, doc(cfg(feature = "tree")))]
pub mod tree;
mod vtable;

#[cfg(feature = "tree")]
#[doc(inline)]
#[cfg_attr(docrs, doc(cfg(feature = "tree")))]
pub use tree::{Stk as TreeStk, TreeStack};

#[doc(inline)]
pub use stack::{Stack, Stk};

#[cfg(test)]
mod test;
