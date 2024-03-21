#![allow(dead_code)]

mod allocator;
pub mod stack;
mod stub_ctx;
mod task;

#[cfg(feature = "tree")]
pub mod tree;

#[cfg(feature = "tree")]
#[doc(inline)]
pub use tree::{Stk as TreeStk, TreeStack};

#[doc(inline)]
pub use stack::{Stack, Stk};

#[cfg(test)]
mod test;
