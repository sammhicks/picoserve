[![crates.io](https://img.shields.io/crates/v/const-sha1.svg)](https://crates.io/crates/const-sha1)
[![docs.rs](https://docs.rs/const-sha1/badge.svg)](https://docs.rs/const-sha1/)
[![Build and Test](https://github.com/rylev/const-sha1/workflows/Build%20and%20Test/badge.svg?event=push)](https://github.com/rylev/const-sha1/actions)

# const-sha1

A sha1 implementation useable in const contexts. 

## Use

 ```rust
 const fn signature() -> [u32; 5] {
     const_sha1::sha1(stringify!(MyType).as_bytes()).data
 }
 ```

# Minimum Supported Rust Version (MSRV)

This crate requires Rust 1.46.0 or newer due to the use of some const expression features.

# No-std

```
const-sha1 = { version = "0.2.0", default-features = false }
```

## Attribution

This code is largely inspired by the following repos:
* [vog/sha1](https://github.com/vog/sha1)
* [mitsuhiko/rust-sha1](https://github.com/mitsuhiko/rust-sha1)