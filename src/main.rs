use baremetal_ipc_sockets::{recv_fd, send_fd, RingBuffer};
use std::io::{Read, Write};
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::os::unix::net::UnixStream;
use std::time::Instant;

fn main() {
    let (a, b) = UnixStream::pair().expect("socketpair");

    let (r, w) = UnixStream::pair().expect("auxiliary socketpair");
    let start = Instant::now();
    send_fd(&a, w.as_raw_fd()).expect("send fd");
    let transferred = recv_fd(&b).expect("recv fd");
    let transfer_elapsed = start.elapsed();

    let mut peer = unsafe { UnixStream::from_raw_fd(transferred) };
    peer.write_all(&[0xAA]).expect("write over transferred fd");
    let mut byte = [0u8; 1];
    r.read_exact(&mut byte).expect("read from peer");
    assert_eq!(byte[0], 0xAA, "transferred fd must share the socket");

    let ring = RingBuffer::with_capacity(4);
    let start = Instant::now();
    for i in 0..8u64 {
        ring.push(i).expect("ring full");
    }
    let mut popped = Vec::new();
    while let Some(value) = ring.pop() {
        popped.push(value);
    }
    let ring_elapsed = start.elapsed();

    println!(
        "fd_transfer={:?} bytes_ok=1 ring_items=8 ring_elapsed={:?}",
        transfer_elapsed, ring_elapsed
    );
    assert_eq!(popped, (0..8u64).collect::<Vec<_>>());
}
