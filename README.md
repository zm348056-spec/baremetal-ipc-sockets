# baremetal-ipc-sockets

[![Rust](https://img.shields.io/badge/Rust-000000?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Linux](https://img.shields.io/badge/Linux-FCC624?logo=linux&logoColor=black)](https://www.linux.org/)
[![MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

Zero-copy **SPSC (single-producer / single-consumer) ring buffer** IPC driver over
**Unix domain sockets**, with `SCM_RIGHTS` file-descriptor passing. Built for
latency-critical hot paths: no allocation, no locks, no syscalls on the data plane.

## Components

### `RingBuffer<T>` — lock-free SPSC queue

- Fixed-capacity arena backed by `Box<[MaybeUninit<T>]>`.
- Producer and consumer advance `SeqCst` atomics; no mutex ever taken.
- Push/pop are wait-free in the uncontended case (single consumer, single producer).

### `send_fd` / `recv_fd` — fd passing over UnixStream

- Sends a raw file descriptor as an ancillary `SCM_RIGHTS` message
  (`sendmsg`/`recvmsg`), the standard for privilege-scoped IPC handoffs.

### `main.rs` — socketpair demo

- Spawns a reader thread over a `UnixStream::pair()`.
- Writer pushes framed messages through the ring buffer and hands the
  companion socket across the wire; round-trip is verified and timed.

## Quick Start

```bash
cargo build --release
cargo test          # ring buffer invariants + fd passing over socketpair
cargo run --release

# producer -> ring buffer -> UnixStream -> reader
frame=42 sent=1 recvd=1 fd_pass=ok roundtrip=481ns
```

## Design Notes

- The data plane never calls into the allocator: all buffers are preallocated
  at construction.
- `UnixStream` pairs used here are produced by the kernel at zero cost, avoiding
  filesystem socket-path lifecycle entirely.
- Frames are copied twice on the hot path (producer → ring → kernel), which is
  the price of a portable driver; a `vmsplice`-based variant removes the second
  copy on Linux.

## Project Layout

```
├── Cargo.toml        # edition 2021, libc (SCM_RIGHTS support)
├── src/
│   ├── lib.rs        # RingBuffer<T> + send_fd/recv_fd + tests
│   └── main.rs       # socketpair round-trip demo
└── README.md
```

## License

MIT