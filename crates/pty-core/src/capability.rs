//! Capability-channel transport over Unix socket ancillary data.
//!
//! A controller holds one end of a private `socketpair`. Readiness requests
//! pass that exact end with `SCM_RIGHTS`; the daemon challenges the holder on
//! its already-inherited opposite end before accepting a control request.

use std::io;
use std::mem::size_of;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;

fn aligned(len: usize) -> usize {
    let alignment = size_of::<usize>();
    (len + alignment - 1) & !(alignment - 1)
}

fn control_space() -> usize {
    aligned(size_of::<libc::cmsghdr>() + size_of::<RawFd>())
}

/// Send one marker byte and one owned descriptor through `stream`.
pub fn send_fd(stream: &UnixStream, fd: RawFd) -> io::Result<()> {
    let mut marker = [0u8; 1];
    let mut iov = libc::iovec {
        iov_base: marker.as_mut_ptr().cast(),
        iov_len: marker.len(),
    };
    let mut control = vec![0u8; control_space()];
    unsafe {
        let header = control.as_mut_ptr().cast::<libc::cmsghdr>();
        (*header).cmsg_len = size_of::<libc::cmsghdr>() + size_of::<RawFd>();
        (*header).cmsg_level = libc::SOL_SOCKET;
        (*header).cmsg_type = libc::SCM_RIGHTS;
        let payload = header.cast::<u8>().add(size_of::<libc::cmsghdr>()).cast::<RawFd>();
        *payload = fd;
    }
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = std::ptr::addr_of_mut!(iov);
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = control.len();
    let result = unsafe { libc::sendmsg(stream.as_raw_fd(), &message, 0) };
    if result == 1 {
        Ok(())
    } else if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Err(io::Error::new(io::ErrorKind::WriteZero, "capability marker not sent"))
    }
}

/// Receive one marker byte and at most one passed descriptor.
pub fn recv_fd(stream: &UnixStream, marker: &mut [u8; 1]) -> io::Result<(usize, Option<OwnedFd>)> {
    let mut iov = libc::iovec {
        iov_base: marker.as_mut_ptr().cast(),
        iov_len: marker.len(),
    };
    let mut control = vec![0u8; control_space()];
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = std::ptr::addr_of_mut!(iov);
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = control.len();
    let result = unsafe { libc::recvmsg(stream.as_raw_fd(), &mut message, 0) };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut received = None;
    if message.msg_controllen >= size_of::<libc::cmsghdr>() {
        let header = unsafe { control.as_ptr().cast::<libc::cmsghdr>().as_ref() };
        if let Some(header) = header
            && header.cmsg_level == libc::SOL_SOCKET
            && header.cmsg_type == libc::SCM_RIGHTS
            && header.cmsg_len >= size_of::<libc::cmsghdr>() + size_of::<RawFd>()
        {
            let raw = unsafe {
                control
                    .as_ptr()
                    .add(size_of::<libc::cmsghdr>())
                    .cast::<RawFd>()
                    .read()
            };
            if raw >= 0 {
                let owned = unsafe { OwnedFd::from_raw_fd(raw) };
                unsafe { libc::fcntl(owned.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
                received = Some(owned);
            }
        }
    }
    Ok((result as usize, received))
}

/// Create the two-endpoint capability channel used by `pty ctl exec`.
pub fn socketpair() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [-1; 2];
    if unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_an_exact_descriptor_over_the_private_channel() {
        let (sender, receiver) = socketpair().expect("socketpair");
        let sender = UnixStream::from(sender);
        let receiver = UnixStream::from(receiver);
        let source = std::fs::File::open("/dev/null").expect("/dev/null");
        send_fd(&sender, source.as_raw_fd()).expect("send capability fd");
        let mut marker = [0u8; 1];
        let (length, received) = recv_fd(&receiver, &mut marker).expect("receive capability fd");
        assert_eq!(length, 1);
        assert!(received.is_some());
        assert_eq!(marker, [0]);
    }
}
