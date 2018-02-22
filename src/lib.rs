// Copyright 2017 Dasein Phaos aka. Luxko
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! A thread-local memory allocator backed up by the concept of object pools,
//! used to address the memory allocation needs of `arendur`.
//!
//! This crate is useful when
//!
//! - you want to frequently create and destroy some objects
//! - these objects are copyable small ones, with size under 256 bytes
//! - you want the underlying memory to be reused
//! - you want a unified interface for the pool, rather than a typed one
//!
//! # Usage
//!
//! ```rust
//! use aren_alloc::Allocator;
//! #[derive(Copy, Clone)]
//! struct Point(u32, u32);
//! let allocator = Allocator::new();
//! let p = allocator.alloc(Point(1, 2));
//! assert_eq!(p.0, 1);
//! assert_eq!(p.1, 2);
//! ```

#![feature(coerce_unsized)]
#![feature(unsize)]

use std::cell::{Cell, RefCell};
use std::marker::Unsize;
use std::ops::CoerceUnsized;

#[derive(Copy, Clone)]
struct Node {
    next: *mut Node,
}

struct Pool {
    pool: RefCell<Vec<u8>>,
    head: Cell<*mut Node>,
    ele_size: usize,
    next_pool: RefCell<Option<Box<Pool>>>,
    tail_pool: Cell<*mut Pool>,
}

const DEFAULT_POOL_SIZE: usize = 4096;

impl Pool {
    fn new(ele_size: usize) -> Box<Pool> {
        debug_assert!(DEFAULT_POOL_SIZE%ele_size==0);
        debug_assert!(ele_size<=DEFAULT_POOL_SIZE);
        Pool::with_capacity(DEFAULT_POOL_SIZE/ele_size, ele_size)
    }

    fn with_capacity(num: usize, ele_size: usize) -> Box<Pool> {
        debug_assert!(num>0);
        debug_assert!(ele_size>=std::mem::size_of::<Node>());
        debug_assert!(ele_size.is_power_of_two());

        let mut pool: Vec<u8> = Vec::with_capacity(num*ele_size);
        let head: *mut Node = unsafe {
            let head = pool.as_mut_ptr();
            for i in 0..num-1 {
                let cur = head.offset((i*ele_size) as isize).as_mut().unwrap();
                let next = head.offset(((i+1)*ele_size) as isize);
                let cur: *mut Node = std::mem::transmute(cur);
                cur.as_mut().unwrap().next = std::mem::transmute(next);
            }
            let tail = head.offset(((num-1)*ele_size) as isize);
            let tail: *mut Node = std::mem::transmute(tail);
            tail.as_mut().unwrap().next = std::ptr::null_mut();
            std::mem::transmute(head)
        };
        
        let mut p = Box::new(Pool{
            pool: RefCell::new(pool),
            head: Cell::new(head),
            ele_size: ele_size,
            next_pool: RefCell::new(None),
            tail_pool: Cell::new(std::ptr::null_mut()),
        });
        let pmut = <Box<_> as std::ops::DerefMut>::deref_mut(&mut p) as *mut Pool;
        p.tail_pool.set(pmut);
        p
    }

    fn alloc<T>(&self) -> Pointer<T> {
        debug_assert!(std::mem::size_of::<T>() <= self.ele_size);
        debug_assert!(self.ele_size%std::mem::align_of::<T>() == 0);
        // if std::mem::size_of::<T>() <= 16 || self.head.get().is_null() {
        if self.head.get().is_null() {
            self.extend();
        }
        debug_assert!(!self.head.get().is_null());
        let lasthead = self.head.get();
        let nexthead = unsafe {lasthead.as_mut().unwrap().next};
        self.head.set(nexthead);
        unsafe {Pointer{
            pool: self, node: std::mem::transmute(lasthead)
        }}
    }

    fn extend(&self) {
        if self.head.get().is_null() { unsafe {
            let tail = self.tail_pool.get().as_mut().unwrap();
            debug_assert!(tail.next_pool.borrow().is_none());
            let num = self.pool.borrow().capacity() / self.ele_size;
            let mut next_pool = Pool::with_capacity(num, self.ele_size);
            {
                let newtail = <Box<_> as std::ops::DerefMut>::deref_mut(&mut next_pool);
                self.head.set(newtail.head.get());
                self.tail_pool.set(newtail);
            }
            *tail.next_pool.get_mut() = Some(next_pool);
        }}
    }

    unsafe fn recycle(&self, node: *mut Node) {
        debug_assert!(!node.is_null());
        let oldhead = self.head.get();
        let noderef = node.as_mut().unwrap();
        noderef.next = oldhead;
        self.head.set(node);
    }
}

/// A pointer to `T`, when dropped, the underlying memory
/// would be recycled by the allocator.
pub struct Pointer<'a, T: ?Sized> {
    pool: &'a Pool,
    node: *mut T,
}

impl<'a, T, U> CoerceUnsized<Pointer<'a, T>> for Pointer<'a, U>
    where U: Unsize<T> + ?Sized,
          T: ?Sized,
{ }

impl<'a, T: ?Sized> Pointer<'a, T> {
    /// Borrow `ptr` as a reference.
    /// This is an associated function so that
    /// `T`'s methods won't be shadowed.
    #[inline]
    pub fn as_ref(ptr: &Self) -> &T {
        unsafe {
            &*ptr.node
        }
    }

    /// Borrow `ptr` as a mutable reference.
    /// This is an associated function so that
    /// `T`'s methods won't be shadowed.
    #[inline]
    pub fn as_mut(ptr: &mut Self) -> &mut T {
        unsafe {
            &mut *ptr.node
        }
    }

    // /// Borrow `ptr` as a mutable reference,
    // /// return a typed erased pointer with it.
    // ///
    // /// This is useful when casting the `&mut T` to some trait objects
    // #[inline]
    // pub fn erase_borrow_mut(ptr: Self) -> (&'a mut T, Pointer<'a, ()>) {
    //     debug_assert!(!ptr.node.is_null());
    //     unsafe {
    //         let tptr: *mut T = std::mem::transmute(ptr.node);
    //         (tptr.as_mut().unwrap(), Pointer{pool: ptr.pool, node: ptr.node, _phantom: Default::default()})
    //     }
    // }
}

impl<'a, T:?Sized> std::ops::Deref for Pointer<'a, T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        Pointer::as_ref(self)
    }
}

impl<'a, T:?Sized> std::ops::DerefMut for Pointer<'a, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        Pointer::as_mut(self)
    }
}

impl<'a, T:?Sized> Drop for Pointer<'a, T> {
    fn drop(&mut self) {
        unsafe {
            let node: *mut Node = std::mem::transmute_copy(&self.node);
            self.pool.recycle(node);
        }
    }
}       

/// Allows allocation
pub struct Allocator {
    pool8: Box<Pool>,
    pool16: Box<Pool>,
    pool32: Box<Pool>,
    pool64: Box<Pool>,
    pool128: Box<Pool>,
    pool256: Box<Pool>,
}

impl Allocator {
    /// Construct a new allocator with default page capacity.
    pub fn new() -> Allocator {
        Allocator{
            pool8: Pool::new(8),
            pool16: Pool::new(16),
            pool32: Pool::new(32),
            pool64: Pool::new(64),
            pool128: Pool::new(128),
            pool256: Pool::new(256),
        }
    }

    /// Construct a new allocator with `cap`acity per inner page
    pub fn with_capacity(cap: usize) -> Allocator {
        Allocator{
            pool8: Pool::with_capacity(cap, 8),
            pool16: Pool::with_capacity(cap, 16),
            pool32: Pool::with_capacity(cap, 32),
            pool64: Pool::with_capacity(cap, 64),
            pool128: Pool::with_capacity(cap, 128),
            pool256: Pool::with_capacity(cap, 256),
        }
    }

    /// Allocate an instance of `T` with value `elem`,
    /// return the allocated pointer.
    /// `size_of::<T>()` should be le to 256 bytes.
    #[inline]
    pub fn alloc<T: Copy>(&self, elem: T) -> Pointer<T> {
        let ele_size = std::mem::size_of::<T>();
        let mut ret = if ele_size <= 8 {
            self.pool8.alloc()
        } else if ele_size <= 16 {
            self.pool16.alloc()
        } else if ele_size <= 32 {
            self.pool32.alloc()
        } else if ele_size <= 64 {
            self.pool64.alloc()
        } else if ele_size <= 128 {
            self.pool128.alloc()
        } else if ele_size <= 256 {
            self.pool256.alloc()
        } else {
            panic!("element size too big!");
        };

        *ret = elem;
        ret
    }

    /// Allocate an instance of `T` with default value,
    /// return the allocated pointer.
    /// `size_of::<T>()` should be le to 256 bytes.
    #[inline]
    pub fn alloc_default<T: Copy+Default>(&self) -> Pointer<T> {
        self.alloc(Default::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Copy, Clone, Eq, PartialEq, Debug)]
    struct Byte128 {
        val: [u64; 16]
    }

    impl Byte128 {
        fn new(v: u64) -> Byte128 {
            Byte128{ val: [v; 16] }
        }
    }

    impl Default for Byte128 {
        fn default() -> Byte128 {
            Byte128::new(0)
        }
    }

    #[derive(Copy, Clone, Eq, PartialEq, Debug)]
    struct Byte15 {
        val: [u8; 15]
    }

    impl Byte15 {
        fn new(v: u8) -> Byte15 {
            Byte15{ val: [v; 15] }
        }
    }

    impl Default for Byte15 {
        fn default() -> Byte15 {
            Byte15::new(0)
        }
    }

    trait Sum {
        fn sum(&self) -> u64;
    }

    impl Sum for Byte15 {
        fn sum(&self) -> u64 {
            let mut ret = 0;
            for i in self.val.iter() {
                ret += *i as u64;
            }
            ret
        }
    }

    #[test]
    fn test_alloc_16() {
        let allocator = Allocator::new();
        let bytes1 = allocator.alloc(Byte15::new(1));
        let bytes2 = allocator.alloc(Byte15::new(2));
        let bytes3 = allocator.alloc(Byte15::new(3));
        {
            let bytes4 = allocator.alloc(Byte15::new(4));
            assert_eq!(*bytes1, Byte15::new(1));
            assert_eq!(*bytes2, Byte15::new(2));
            assert_eq!(*bytes3, Byte15::new(3));
            assert_eq!(*bytes4, Byte15::new(4));
        }
        assert_eq!(*bytes1, Byte15::new(1));
        assert_eq!(*bytes2, Byte15::new(2));
        assert_eq!(*bytes3, Byte15::new(3));
    }

    #[test]
    #[should_panic]
    fn test_alloc_32_panic() {
        let allocator = Pool::new(32);
        let _bytes128: Pointer<Byte128> = allocator.alloc();
    }

    #[test]
    fn test_alloc_128_addtional_pages() {
        let allocator = Allocator::with_capacity(4);
        let bytes1 = allocator.alloc(Byte128::new(1));
        let bytes2 = allocator.alloc(Byte128::new(2));
        let bytes3 = allocator.alloc(Byte128::new(3));
        {
            let bytes4 = allocator.alloc(Byte128::new(4));
            let bytes5 = allocator.alloc(Byte128::new(5));
            let bytes6 = allocator.alloc(Byte128::new(6));
            let bytes7 = allocator.alloc(Byte128::new(7));
            let bytes8 = allocator.alloc(Byte128::new(8));
            let bytes9 = allocator.alloc(Byte128::new(9));
            let bytes10 = allocator.alloc(Byte128::new(10));
            assert_eq!(*bytes1, Byte128::new(1));
            assert_eq!(*bytes2, Byte128::new(2));
            assert_eq!(*bytes3, Byte128::new(3));
            assert_eq!(*bytes4, Byte128::new(4));
            assert_eq!(*bytes5, Byte128::new(5));
            assert_eq!(*bytes6, Byte128::new(6));
            assert_eq!(*bytes7, Byte128::new(7));
            assert_eq!(*bytes8, Byte128::new(8));
            assert_eq!(*bytes9, Byte128::new(9));
            assert_eq!(*bytes10, Byte128::new(10));
        }
        let bytes6 = allocator.alloc(Byte128::new(6));
        let bytes7 = allocator.alloc(Byte128::new(7));
        let bytes8 = allocator.alloc(Byte128::new(8));
        let bytes9 = allocator.alloc(Byte128::new(9));
        let bytes10 = allocator.alloc(Byte128::new(10));
        assert_eq!(*bytes1, Byte128::new(1));
        assert_eq!(*bytes2, Byte128::new(2));
        assert_eq!(*bytes3, Byte128::new(3));
        assert_eq!(*bytes6, Byte128::new(6));
        assert_eq!(*bytes7, Byte128::new(7));
        assert_eq!(*bytes8, Byte128::new(8));
        assert_eq!(*bytes9, Byte128::new(9));
        assert_eq!(*bytes10, Byte128::new(10));
    }

    #[test]
    fn test_alloc_default() {
        let allocator = Allocator::new();
        let d: Pointer<Byte128> = allocator.alloc_default();
        assert_eq!(*d, Byte128::default());
        let di: Pointer<i32> = allocator.alloc_default();
        assert_eq!(*di, i32::default());
    }

    #[test]
    fn test_as_ref_mut() {
        let allocator = Allocator::new();
        let mut bytes1 = allocator.alloc(Byte15::new(1));
        assert_eq!(bytes1.val[0], 1);
        bytes1.val[1] = 2;
        assert_eq!(bytes1.val[1], 2);
    }

    #[test]
    fn test_unsize_coerce() {
        let allocator = Allocator::new();
        let bytes0 = allocator.alloc(Byte15::new(0));
        {
            let bytes1: Pointer<Sum> = allocator.alloc(Byte15::new(1));
            assert_eq!(bytes1.sum(), 15);
        }
        let bytes2 = allocator.alloc(Byte15::new(2));
        let bytes3 = allocator.alloc(Byte15::new(3));
        assert_eq!(bytes0.sum(), 0);
        assert_eq!(bytes2.sum(), 30);
        assert_eq!(bytes3.sum(), 45);
    }
}
