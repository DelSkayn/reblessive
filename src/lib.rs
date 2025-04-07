#![cfg_attr(docrs, feature(doc_cfg))]

mod allocator;
mod defer;
mod ptr;
mod stub_waker;
mod vtable;

pub mod stack;

#[cfg(feature = "tree")]
#[cfg_attr(docrs, doc(cfg(feature = "tree")))]
pub mod tree;

#[cfg(feature = "tree")]
#[doc(inline)]
#[cfg_attr(docrs, doc(cfg(feature = "tree")))]
pub use tree::{Stk as TreeStk, TreeStack};

#[doc(inline)]
pub use stack::{Stack, Stk};

#[cfg(test)]
mod test;
