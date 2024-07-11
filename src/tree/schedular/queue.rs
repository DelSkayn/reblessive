use std::{
    cell::Cell,
    pin::Pin,
    ptr::{self},
    sync::atomic::{AtomicPtr, Ordering},
};

use crate::ptr::Owned;

use super::atomic_waker::AtomicWaker;

pub struct NodeHeader {
    next: AtomicPtr<NodeHeader>,
}

impl NodeHeader {
    pub fn new() -> NodeHeader {
        NodeHeader {
            next: AtomicPtr::new(ptr::null_mut()),
        }
    }
}

pub struct Queue {
    waker: AtomicWaker,
    head: AtomicPtr<NodeHeader>,
    tail: Cell<Owned<NodeHeader>>,
    stub: NodeHeader,
}

pub enum Pop {
    Empty,
    Value(Owned<NodeHeader>),
    Inconsistant,
}

/// Intrusive MPSC queue from 1024cores blog.
/// Similar to the one used int the FuturesUnordered implementation
impl Queue {
    pub fn new() -> Self {
        Queue {
            waker: AtomicWaker::new(),
            head: AtomicPtr::new(ptr::null_mut()),
            tail: Cell::new(Owned::dangling()),
            stub: NodeHeader {
                next: AtomicPtr::new(ptr::null_mut()),
            },
        }
    }

    pub fn waker(&self) -> &AtomicWaker {
        &self.waker
    }

    pub unsafe fn init(self: Pin<&Self>) {
        let ptr = Owned::from(&self.stub);
        self.head.store(ptr.as_ptr(), Ordering::Release);
        self.tail.set(ptr);
    }

    /// # Safety
    /// - node must be a valid pointer
    /// - Queue must have been properly initialized.
    pub unsafe fn push(self: Pin<&Self>, node: Owned<NodeHeader>) {
        node.as_ref().next.store(ptr::null_mut(), Ordering::Release);

        let prev = self.get_ref().head.swap(node.as_ptr(), Ordering::AcqRel);

        (*prev).next.store(node.as_ptr(), Ordering::Release);
    }

    /// # Safety
    /// - Queue must have been properly initialized.
    /// - Can only be called from a single thread.
    pub unsafe fn pop(self: Pin<&Self>) -> Pop {
        let mut tail = self.tail.get();
        let mut next = Owned::from_ptr(tail.as_ref().next.load(Ordering::Acquire));

        if tail == Owned::from(&self.get_ref().stub) {
            let Some(n) = next else {
                return Pop::Empty;
            };

            self.tail.set(n);
            tail = n;
            next = Owned::from_ptr(n.as_ref().next.load(std::sync::atomic::Ordering::Acquire));
        }

        if let Some(n) = next {
            self.tail.set(n);
            return Pop::Value(tail);
        }

        let head = Owned::from_ptr(self.head.load(Ordering::Acquire));
        if head != Some(tail) {
            return Pop::Inconsistant;
        }

        self.push(Owned::from(&self.get_ref().stub));

        next = Owned::from_ptr(tail.as_ref().next.load(Ordering::Acquire));

        if let Some(n) = next {
            self.tail.set(n);
            return Pop::Value(tail);
        }

        return Pop::Empty;
    }
}
