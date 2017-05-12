// Copyright 2017 Dasein Phaos aka. Luxko
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! A memory allocator backed up by the concept of object pools,
//! used to back up the memory allocation needs of `arendur`.
//!
//! This crate is useful when
//!
//! - you want to frequently create and destroy some objects
//! - these objects are copyable small ones, under some 250 bytes
//! - you want the underlying memory to be reused
//! - you want a unified interface for the pool, rather than a typed one

#![feature(untagged_unions)]

use std::marker::PhantomData;
use std::cell::{Cell, RefCell};

#[derive(Copy, Clone)]
struct Byte16 {
    val: [u8; 16],
}

#[derive(Copy, Clone)]
struct Byte32 {
    val: [Byte16; 2],
}

#[derive(Copy, Clone)]
struct Byte64 {
    val: [Byte16; 4],
}

#[derive(Copy, Clone)]
struct Byte128 {
    val: [Byte16; 8],
}

#[derive(Copy, Clone)]
struct Byte256 {
    val: [Byte16; 16],
}

#[derive(Copy, Clone)]
union Node16 {
    val: Byte16,
    next: *mut Node16,
}

trait Pool {
    type Node;
    unsafe fn recycle(&self, node: *mut Self::Node);
}

struct Pool16 {
    pool: RefCell<Vec<Node16>>,
    head: Cell<*mut Node16>,
    // next: Option<Box<Pool16>>,
}

impl Pool16 {
    fn new() -> Pool16 {
        Pool16::with_capacity(4096/16)
    }

    fn with_capacity(num: usize) -> Pool16 {
        assert!(num>0);
        let mut pool: Vec<Node16> = Vec::with_capacity(num);
        let head = unsafe {
            let head = pool.as_mut_ptr();
            for i in 0..num-1 {
                let cur = head.offset(i as isize).as_mut().unwrap();
                let next = head.offset((i+1) as isize);
                cur.next = next;
            }
            head.offset((num-1) as isize).as_mut().unwrap().next = std::ptr::null_mut();
            head
        };
        Pool16{
            pool: RefCell::new(pool),
            head: Cell::new(head),
            // next: None,
        }
    }

    // fn append_new_pool(&mut self) {
    //     let cap = self.pool.capacity();
    //     self.next = Some(Box::new(
    //         Pool16::with_capacity(cap)
    //     ));
    // }

    fn next_ptr<T>(&self) -> Option<PoolPtr<Pool16, Node16, T>> {
        debug_assert!(std::mem::size_of::<T>() <= 16);
        // if std::mem::size_of::<T>() <= 16 || self.head.get().is_null() {
        if self.head.get().is_null() {
            None
        } else {
            let lasthead = self.head.get();
            let nexthead = unsafe {lasthead.as_mut().unwrap().next};
            self.head.set(nexthead);
            Some(PoolPtr{
                pool: self, node: lasthead, _phantom: Default::default()
            })
        }
    }
}

impl Pool for Pool16 {
    type Node = Node16;
    unsafe fn recycle(&self, node: *mut Node16) {
        debug_assert!(!node.is_null());
        let noderef = node.as_mut().unwrap();
        noderef.next = self.head.get();
        self.head.set(node);
    }
}

struct PoolPtr<'a, P: 'a + Pool<Node=N>, N: 'a, T> {
    pool: &'a P,
    node: *mut N,
    _phantom: PhantomData<T>,
}

impl<'a, P: 'a + Pool<Node=N>, N:'a, T> PoolPtr<'a, P, N, T> {
    fn as_ref(&self) -> &T {unsafe {
        let tptr: *mut T = std::mem::transmute(self.node);
        tptr.as_ref().unwrap()
    }}

    fn as_mut(&mut self) -> &mut T {unsafe {
        let tptr: *mut T = std::mem::transmute(self.node);
        tptr.as_mut().unwrap()
    }}
}

impl<'a, P, N, T> Drop for PoolPtr<'a, P, N, T>
    where P: Pool<Node=N>,
{
    fn drop(&mut self) {unsafe {
        self.pool.recycle(self.node);
    }}
}

#[derive(Copy, Clone)]
struct NodeB {
    next: *mut NodeB,
}

struct PoolB {
    pool: RefCell<Vec<u8>>,
    head: Cell<*mut NodeB>,
    ele_size: usize,
}

impl PoolB {
    fn new(ele_size: usize) -> PoolB {
        debug_assert!(ele_size.is_power_of_two());
        debug_assert!(ele_size<=4096);
        PoolB::with_capacity(4096/ele_size, ele_size)
    }

    fn with_capacity(num: usize, ele_size: usize) -> PoolB {
        debug_assert!(num>0);
        debug_assert!(ele_size.is_power_of_two());

        let mut pool: Vec<u8> = Vec::with_capacity(num*ele_size);
        let head = unsafe {
            let head = pool.as_mut_ptr();
            let head: *mut NodeB = std::mem::transmute(head);
            for i in 0..num-1 {
                let cur = head.offset(i as isize).as_mut().unwrap();
                let next = head.offset((i+1) as isize);
                cur.next = next;
            }
            head.offset((num-1) as isize).as_mut().unwrap().next = std::ptr::null_mut();
            head
        };
        PoolB{
            pool: RefCell::new(pool),
            head: Cell::new(head),
            ele_size: ele_size,
        }
    }

    fn alloc<T>(&self) -> Option<Pointer<T>> {
        debug_assert!(std::mem::size_of::<T>() <= self.ele_size);
        // if std::mem::size_of::<T>() <= 16 || self.head.get().is_null() {
        if self.head.get().is_null() {
            None
        } else {
            let lasthead = self.head.get();
            let nexthead = unsafe {lasthead.as_mut().unwrap().next};
            self.head.set(nexthead);
            Some(Pointer{
                pool: self, node: lasthead, _phantom: Default::default()
            })
        }
    }

    unsafe fn recycle(&self, node: *mut NodeB) {
        debug_assert!(!node.is_null());
        let oldhead = self.head.get();
        let noderef = node.as_mut().unwrap();
        noderef.next = oldhead;
        self.head.set(node);
    }
}

struct Pointer<'a, T> {
    pool: &'a PoolB,
    node: *mut NodeB,
    _phantom: PhantomData<T>,
}

impl<'a, T> Pointer<'a, T> {
    fn as_ref(&self) -> &T {
        debug_assert!(!self.node.is_null());
        unsafe {
            let tptr: *mut T = std::mem::transmute(self.node);
            tptr.as_ref().unwrap()
        }
    }

    fn as_mut(&mut self) -> &mut T {
        debug_assert!(!self.node.is_null());
        unsafe {
            let tptr: *mut T = std::mem::transmute(self.node);
            tptr.as_mut().unwrap()
        }
    }
}

impl<'a, T> Drop for Pointer<'a, T> {
    fn drop(&mut self) {
        unsafe {self.pool.recycle(self.node); }
    }
}