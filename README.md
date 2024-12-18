# ash-alloc

[![Latest version](https://img.shields.io/crates/a/ash-alloc.svg)](https://crates.io/crates/ash-alloc)
[![Documentation](https://docs.rs/ash-alloc/badge.svg)](https://docs.rs/ash-alloc)
![ZLIB](https://img.shields.io/badge/license-zlib-blue.svg)
![MIT](https://img.shields.io/badge/license-MIT-blue.svg)
![Apache](https://img.shields.io/badge/license-Apache-blue.svg)

A segregated list memory allocator for Vulkan written in Rust, but for ash.
Forked from [vk-alloc](https://github.com/hasenbanck/vk-alloc).

Targets Vulkan 1.2+ using [ash](https://github.com/ash-rs/ash).

## Features

All features are optional by default.

* `tracing` Adds logging using [tracing](https://github.com/tokio-rs/tracing).
* `profiling` Adds support for [profiling](https://github.com/aclysma/profiling).
* `vk-buffer-device-address`: Enables the usage of "vkGetBufferDeviceAddress". Needs the Vulkan
  1.2 "bufferDeviceAddress" device feature enabled.

## License

Licensed under MIT or Apache-2.0 or ZLIB.
