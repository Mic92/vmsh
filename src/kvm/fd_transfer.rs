use log::warn;
use nix::errno::Errno;
use nix::sys::socket::*;
use nix::sys::uio::IoVec;
use simple_error::{bail, try_with};
use std::mem::{size_of, MaybeUninit};
use std::os::unix::prelude::*;
use std::sync::{Arc, RwLock};

use crate::kvm::hypervisor::memory::HvMem;
use crate::kvm::tracee::{socklen_t, Tracee};
use crate::result::Result;
use crate::tracer::inject_syscall;

// inspired by https://github.com/Mic92/cntr/blob/492b2d9e9abc9ccd4f01a0134aab73df16393423/src/ipc.rs
pub struct Socket {
    fd: RawFd,
}

impl Drop for Socket {
    fn drop(&mut self) {
        if let Err(e) = nix::unistd::close(self.fd) {
            warn!("cannot close local socket (fd {}): {}", self.fd, e);
        }
    }
}

impl Socket {
    pub fn new(anon_name: &str) -> Result<Socket> {
        // socket
        let sock = try_with!(
            nix::sys::socket::socket(
                AddressFamily::Unix,
                SockType::Datagram,
                SockFlag::SOCK_CLOEXEC,
                None,
            ),
            "failed to create socket"
        );

        // bind
        let local = try_with!(
            UnixAddr::new_abstract(anon_name.as_bytes()),
            "cannot create abstract addr"
        );
        let ulocal = SockAddr::Unix(local);
        try_with!(
            bind(sock, &ulocal),
            "cannot bind to {:?}",
            local.as_abstract()
        );

        Ok(Socket { fd: sock })
    }

    pub fn connect(&self, anon_name: &str) -> Result<()> {
        let remote = try_with!(
            UnixAddr::new_abstract(anon_name.as_bytes()),
            "cannot create abstract addr"
        );
        let uremote = SockAddr::Unix(remote);
        try_with!(
            connect(self.fd, &uremote),
            "cannot connect to client foobar"
        );

        Ok(())
    }

    pub fn send(&self, messages: &[&[u8]], files: &[RawFd]) -> Result<()> {
        let iov: Vec<IoVec<&[u8]>> = messages.iter().map(|m| IoVec::from_slice(m)).collect();
        let fds: Vec<RawFd> = files.iter().map(|f| f.as_raw_fd()).collect();
        let cmsg = if files.is_empty() {
            vec![]
        } else {
            vec![ControlMessage::ScmRights(&fds)]
        };

        try_with!(
            sendmsg(self.fd, &iov, &cmsg, MsgFlags::empty(), None),
            "sendmsg failed"
        );
        Ok(())
    }

    pub fn receive(
        &self,
        message_length: usize,
        cmsgspace: &mut Vec<u8>,
    ) -> Result<(Vec<u8>, Vec<RawFd>)> {
        let mut msg_buf = vec![0; (message_length) as usize];
        let received;
        let mut files: Vec<RawFd> = Vec::with_capacity(1);
        {
            let iov = [IoVec::from_mut_slice(&mut msg_buf)];
            loop {
                match recvmsg(self.fd, &iov, Some(&mut *cmsgspace), MsgFlags::empty()) {
                    Err(Errno::EAGAIN) | Err(Errno::EINTR) => continue,
                    Err(e) => return try_with!(Err(e), "recvmsg failed"),
                    Ok(msg) => {
                        for cmsg in msg.cmsgs() {
                            if let ControlMessageOwned::ScmRights(fds) = cmsg {
                                for fd in fds {
                                    files.push(fd);
                                }
                            }
                        }
                        received = msg.bytes;
                        break;
                    }
                };
            }
        }
        msg_buf.resize(received, 0);
        Ok((msg_buf, files))
    }
}

pub struct HvSocket {
    fd: libc::c_int,
    tracee: Arc<RwLock<Tracee>>,
}

impl Drop for HvSocket {
    fn drop(&mut self) {
        let tracee = match self.tracee.write() {
            Err(e) => {
                warn!("cannot aquire lock to drop HvSocket: {}", e);
                return;
            }
            Ok(t) => t,
        };
        let proc = match tracee.try_get_proc() {
            Err(e) => {
                warn!("cannot drop HvSocket: {}", e);
                return;
            }
            Ok(t) => t,
        };

        let ret = match proc.close(self.fd) {
            Err(e) => {
                warn!(
                    "cannot execute close socket to drop HvSocket (fd {}): {}",
                    self.fd, e
                );
                return;
            }
            Ok(t) => t,
        };
        if ret != 0 {
            warn!(
                "cannot close hypervisor socket to drop HvSocket (fd {}): {}",
                self.fd, ret
            );
        }
    }
}

impl HvSocket {
    pub fn new(
        tracee: Arc<RwLock<Tracee>>,
        proc: &inject_syscall::Process,
        anon_name: &str,
        addr_local_mem: &HvMem<libc::sockaddr_un>,
    ) -> Result<HvSocket> {
        // socket
        let fd = proc.socket(libc::AF_UNIX, libc::SOCK_DGRAM, 0)?;
        if fd <= 0 {
            // FIXME this fails sometimes with ENOSYS?
            bail!("cannot create socket: {}", nix::errno::from_i32(-fd));
        }
        let server_fd = HvSocket {
            fd,
            tracee: Arc::clone(&tracee),
        };

        // bind
        let local = try_with!(
            UnixAddr::new_abstract(anon_name.as_bytes()),
            "cannot create abstract addr"
        );
        addr_local_mem.write(&local.0)?;
        let addr_len = size_of::<u16>() + local.1;
        let ret = proc.bind(
            server_fd.fd,
            addr_local_mem.ptr as *const libc::sockaddr,
            addr_len as u32,
        )?;
        if ret != 0 {
            let err = -ret as i32;
            bail!("cannot bind: {} (#{})", nix::errno::from_i32(err), ret);
        }

        Ok(server_fd)
    }

    pub fn connect(
        &self,
        proc: &inject_syscall::Process,
        anon_name: &str,
        addr_remote_mem: &HvMem<libc::sockaddr_un>,
    ) -> Result<()> {
        let remote = try_with!(
            UnixAddr::new_abstract(anon_name.as_bytes()),
            "cannot create abstract addr"
        );
        addr_remote_mem.write(&remote.0)?;
        let addr_len = size_of::<u16>() + remote.1;
        let ret = proc.connect(
            self.fd,
            addr_remote_mem.ptr as *const libc::sockaddr,
            addr_len as u32,
        )?;
        if ret < 0 {
            let err = -ret as i32;
            bail!(
                "new_client_remote connect failed: {} (#{})",
                nix::errno::from_i32(err),
                err
            );
        }

        Ok(())
    }

    /// MT: A single message of this type is received. Example: `[u8; 8]`
    /// CM: Control message (cmsg) space. Example: `[0u8; Tracee::CMSG_SPACE((size_of::<RawFd>() * 2) as u32) as _]`
    pub fn receive<MT: Sized + Copy, CM: Sized + Copy>(
        &self,
        proc: &inject_syscall::Process,
        msg_hdr_mem: &HvMem<libc::msghdr>,
        iov_mem: &HvMem<libc::iovec>,
        iov_buf_mem: &HvMem<MT>,
        cmsg_mem: &HvMem<CM>,
    ) -> Result<(MT, Vec<RawFd>)> {
        // init msghdr
        let iov = libc::iovec {
            iov_base: iov_buf_mem.ptr as *mut libc::c_void,
            iov_len: size_of::<MT>(),
        };
        iov_mem.write(&iov)?;

        let mut msg_hdr = MaybeUninit::<msghdr>::zeroed();
        let p = msg_hdr.as_mut_ptr();
        unsafe {
            (*p).msg_name = std::ptr::null_mut::<libc::c_void>();
            (*p).msg_namelen = 0;
            (*p).msg_iov = iov_mem.ptr as *mut libc::iovec;
            (*p).msg_iovlen = 1;
            (*p).msg_control = cmsg_mem.ptr as *mut libc::c_void;
            (*p).msg_controllen = size_of::<CM>() as socklen_t;
            (*p).msg_flags = 0;
        }

        msg_hdr_mem.write(&unsafe { msg_hdr.assume_init() })?;

        // recvmsg
        loop {
            let ret = proc.recvmsg(self.fd, msg_hdr_mem.ptr as *mut libc::msghdr, 0)?;
            if ret == 0 {
                bail!("received empty message");
            }
            if ret < 0 {
                let err = -ret as i32;
                match nix::errno::from_i32(err) {
                    Errno::EAGAIN | Errno::EINTR => continue,
                    e => bail!("recvmsg failed: {} (#{})", e, err),
                }
            }
            break;
        }

        // read message
        let msg_buf = iov_buf_mem.read()?;

        // parse first control message
        let msg_hdr = msg_hdr_mem.read()?;
        let mut cmsg = cmsg_mem.read()?;
        let cmsg_ptr: *mut CM = &mut cmsg;
        let mut result: Vec<RawFd> = vec![];
        unsafe {
            let cmsghdr_ptr: *mut libc::cmsghdr =
                Tracee::__CMSG_FIRSTHDR(cmsg_ptr as *mut libc::c_void, msg_hdr.msg_controllen);
            let cmsghdr: libc::cmsghdr = *cmsghdr_ptr;

            // parse SCM_RIGHTS message
            if cmsghdr.cmsg_type != libc::SCM_RIGHTS {
                bail!("cmsghdr not understood");
            }
            // iterate over SCM_RIGHTS message data
            let cmsg_data: *mut RawFd = Tracee::CMSG_DATA(cmsghdr_ptr) as *mut RawFd;
            let cmsg_data_len =
                cmsghdr.cmsg_len as usize - (cmsg_data as usize - cmsghdr_ptr as usize); // cmsg_size - cmsg_hdr_size
            for offset in 0..(cmsg_data_len / size_of::<RawFd>()) {
                result.push(*(cmsg_data.add(offset)));
            }
        }

        Ok((msg_buf, result))
    }
}
