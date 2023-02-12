//! From https://github.com/ibraheemdev/firefly

#![allow(dead_code)]

use core::{marker::PhantomPinned, pin::Pin, ptr::NonNull};

pub struct LinkedList<T> {
    head: Option<NonNull<Node<T>>>,
    tail: Option<NonNull<Node<T>>>,
}

pub struct Node<T> {
    inner: T,
    prev: Option<NonNull<Node<T>>>,
    next: Option<NonNull<Node<T>>>,
    _pin: PhantomPinned,
}

impl<T> Node<T> {
    #[inline]
    pub const fn new(inner: T) -> Node<T> {
        Node {
            inner,
            next: None,
            prev: None,
            _pin: PhantomPinned,
        }
    }

    #[inline]
    pub fn get(&self) -> &T {
        &self.inner
    }

    #[inline]
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T> LinkedList<T> {
    #[inline]
    pub const fn new() -> Self {
        Self { head: None, tail: None }
    }

    /// add node to head of list.
    ///
    /// # Safety
    ///
    /// node must be removed from the list if the pinned pointer is going invalid(moved, dropped etc).
    pub unsafe fn push_front(&mut self, node: Pin<&mut Node<T>>) {
        let node = node.get_unchecked_mut();
        node.next = self.head;
        node.prev = None;

        if let Some(mut head) = self.head {
            head.as_mut().prev = Some(node.into())
        };

        self.head = Some(node.into());

        if self.tail.is_none() {
            self.tail = Some(node.into());
        }
    }

    /// remove node from tail of list.
    pub fn pop_back(&mut self) -> Option<&mut Node<T>> {
        // SAFETY:
        // trust Self::push_front's caller to provide valid pointer.
        unsafe {
            let mut tail = self.tail?.as_mut();
            self.tail = tail.prev;

            match tail.prev {
                Some(mut prev) => prev.as_mut().next = None,
                None => {
                    debug_assert_eq!(self.head, Some(tail.into()));
                    self.head = None;
                }
            }

            tail.prev = None;
            tail.next = None;
            Some(tail)
        }
    }

    /// removes the given node from the list, returning whether
    /// or not the node was removed.
    ///
    /// # Safety
    ///
    /// node must either be part of this list, or no list at all.
    /// passing a node from another list instance is undefined behavior.
    pub unsafe fn remove(&mut self, node: Pin<&mut Node<T>>) -> bool {
        let node = node.get_unchecked_mut();
        match node.prev {
            None => {
                if self.head != Some(node.into()) {
                    debug_assert!(node.next.is_none());
                    return false;
                }

                self.head = node.next;
            }
            Some(mut prev) => {
                debug_assert_eq!(prev.as_ref().next, Some(node.into()));
                prev.as_mut().next = node.next;
            }
        }

        match node.next {
            Some(mut next) => {
                debug_assert_eq!(next.as_mut().prev, Some(node.into()));
                next.as_mut().prev = node.prev;
            }
            None => {
                debug_assert_eq!(self.tail, Some(node.into()));
                self.tail = node.prev;
            }
        }

        node.next = None;
        node.prev = None;

        true
    }
}
