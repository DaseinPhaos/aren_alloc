# aren_alloc

![travis](https://travis-ci.org/DaseinPhaos/aren_alloc.svg?branch=master)
[![Crates.io](https://img.shields.io/crates/v/aren_alloc.svg)](https://crates.io/crates/aren_alloc)

A thread-local memory allocator backed up by the concept of object pools,
used to address the memory allocation needs of `arendur`.

This crate is useful when

- you want to frequently create and destroy some objects
- these objects are copyable small ones, with size under 256 bytes
- you want the underlying memory to be reused
- you want a unified interface for the pool, rather than a typed one

# Usage

```rust
use aren_alloc::Allocator;
#[derive(Copy, Clone)]
struct Point(u32, u32);
let allocator = Allocator::new();
let p = allocator.alloc(Point(1, 2));
assert_eq!(p.0, 1);
assert_eq!(p.1, 2);
```

# License

This project is distributed under the terms of both the MIT license and the Apache License (Version 2.0).

See [LICENSE-APACHE](LICENSE-APACHE), [LICENSE-MIT](LICENSE-MIT) for details.
