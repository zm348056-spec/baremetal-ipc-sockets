use std::mem::MaybeUninit;
use std::os::unix::io::RawFd;
use std::os::unix::net::UnixStream;
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct RingBuffer<T> {
    buf: Box<[MaybeUninit<T>]>,
    mask: usize,
    head: AtomicUsize,
    tail: AtomicUsize,
}

impl<T> RingBuffer<T> {
    pub fn with_capacity(capacity: usize) -> Self {
        assert!(capacity.is_power_of_two() && capacity >= 2);
        let mut buf = Vec::with_capacity(capacity);
        buf.resize_with(capacity, MaybeUninit::uninit);
        Self {
            buf: buf.into_boxed_slice(),
            mask: capacity - 1,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    pub fn len(&self) -> usize {
        self.head
            .load(Ordering::SeqCst)
            .wrapping_sub(self.tail.load(Ordering::SeqCst))
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn push(&self, value: T) -> Result<(), T> {
        let head = self.head.load(Ordering::SeqCst);
        let tail = self.tail.load(Ordering::SeqCst);
        if head.wrapping_sub(tail) == self.buf.len() {
            return Err(value);
        }
        unsafe {
            self.buf[head & self.mask].write(value);
        }
        self.head.store(head.wrapping_add(1), Ordering::SeqCst);
        Ok(())
    }

    pub fn pop(&self) -> Option<T> {
        let tail = self.tail.load(Ordering::SeqCst);
        let head = self.head.load(Ordering::SeqCst);
        if tail == head {
            return None;
        }
        let value = unsafe { self.buf[tail & self.mask].assume_init_read() };
        self.tail.store(tail.wrapping_add(1), Ordering::SeqCst);
        Some(value)
    }
}

impl<T> Drop for RingBuffer<T> {
    fn drop(&mut self) {
        let head = self.head.load(Ordering::SeqCst);
        let mut tail = self.tail.load(Ordering::SeqCst);
        while tail != head {
            unsafe {
                self.buf[tail & self.mask].assume_init_drop();
            }
            tail = tail.wrapping_add(1);
        }
    }
}

unsafe impl<T: Send> Send for RingBuffer<T> {}
unsafe impl<T: Sync> Sync for RingBuffer<T> {}

pub fn send_fd(stream: &UnixStream, fd: RawFd) -> std::io::Result<()> {
    let iov = libc::iovec {
        iov_base: ptr::null_mut(),
        iov_len: 0,
    };
    let mut cmsg = [0u8; 64];
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &iov as *const libc::iovec as *mut libc::iovec;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg.as_mut_ptr() as *mut libc::c_void;
    msg.msg_controllen = cmsg.len();
    unsafe {
        let header = msg.msg_control as *mut libc::cmsghdr;
        (*header).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<libc::c_int>());
        (*header).cmsg_level = libc::SOL_SOCKET;
        (*header).cmsg_type = libc::SCM_RIGHTS;
        ptr::copy_nonoverlapping(
            &fd as *const RawFd as *const u8,
            libc::CMSG_DATA(header) as *mut u8,
            std::mem::size_of::<libc::c_int>(),
        );
        msg.msg_controllen = libc::CMSG_SPACE(std::mem::size_of::<libc::c_int>());
        if libc::sendmsg(stream.as_raw_fd(), &msg, 0) < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

pub fn recv_fd(stream: &UnixStream) -> std::io::Result<RawFd> {
    let mut iov = libc::iovec {
        iov_base: ptr::null_mut(),
        iov_len: 0,
    };
    let mut cmsg = [0u8; 64];
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov as *mut libc::iovec;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg.as_mut_ptr() as *mut libc::c_void;
    msg.msg_controllen = cmsg.len();
    if unsafe { libc::recvmsg(stream.as_raw_fd(), &mut msg, 0) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    unsafe {
        let mut header = libc::CMSG_FIRSTHDR(&msg);
        while !header.is_null() {
            if (*header).cmsg_level == libc::SOL_SOCKET && (*header).cmsg_type == libc::SCM_RIGHTS {
                let fd = ptr::read_unaligned(libc::CMSG_DATA(header) as *const RawFd);
                return Ok(fd);
            }
            header = libc::CMSG_NXTHDR(&msg, header);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::UnexpectedEof,
        "no file descriptor in received message",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::io::{AsRawFd, FromRawFd};

    #[test]
    fn ring_wraps_without_loss() {
        let ring = RingBuffer::with_capacity(4);
        for i in 0..8 {
            ring.push(i).unwrap();
        }
        for i in 0..8 {
            assert_eq!(ring.pop(), Some(i));
        }
        assert_eq!(ring.pop(), None);
    }

    #[test]
    fn ring_full_rejects() {
        let ring = RingBuffer::with_capacity(4);
        for i in 0..4 {
            assert!(ring.push(i).is_ok());
        }
        assert!(ring.push(99).is_err());
    }

    #[test]
    fn fd_crosses_socketpair() {
        let (a, b) = UnixStream::pair().unwrap();
        let (r, w) = UnixStream::pair().unwrap();
        send_fd(&a, w.as_raw_fd()).unwrap();
        let transferred = recv_fd(&b).unwrap();
        let mut peer = unsafe { UnixStream::from_raw_fd(transferred) };
        peer.write_all(&[0xAA]).unwrap();
        let mut byte = [0u8; 1];
        r.read_exact(&mut byte).unwrap();
        assert_eq!(byte[0], 0xAA);
    }
}
