use nix::errno::Errno;
use nix::sys::socket::*;
use nix::sys::uio::IoVec;
use simple_error::{bail, simple_error, try_with};
use std::mem::size_of;
use std::os::unix::prelude::*;

use crate::inject_syscall;
use crate::kvm::{HvMem, Tracee};
use crate::result::Result;

// inspired by https://github.com/Mic92/cntr/blob/492b2d9e9abc9ccd4f01a0134aab73df16393423/src/ipc.rs
pub struct Socket {
    fd: RawFd,
}

// TODO impl drop

impl Socket {
    pub fn new(anon_local_id: u64) -> Result<Socket> {
        println!("new local socket: {}", anon_local_id);
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

        //let mut sun_path = [0u8; 108]; // first byte 0x0 for anonymous socket
        //sun_path[1..9].as_mut().copy_from_slice(&anon_id.to_ne_bytes());
        //let sun_path: [i8; 108] = unsafe { std::mem::transmute::<[u8; 108], [i8; 108]>(sun_path) };
        //let addr = nix::sys::socket::sockaddr_un {
        //sun_family: libc::AF_UNIX as u16,
        //sun_path,
        //};

        // bind
        let local = try_with!(
            UnixAddr::new_abstract(&anon_local_id.to_ne_bytes()),
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

    pub fn new_remote(
        proc: &inject_syscall::Process,
        anon_id_local: u64,
        addr_local_mem: &HvMem<libc::sockaddr_un>,
    ) -> Result<Socket> {
        println!("new remote socket: {}", anon_id_local);
        // socket
        let server_fd = proc.socket(libc::AF_UNIX, libc::SOCK_DGRAM, 0)?;
        if server_fd <= 0 {
            bail!("cannot create socket: {}", server_fd);
        }

        // bind
        let local = try_with!(
            UnixAddr::new_abstract(&anon_id_local.to_ne_bytes()),
            "cannot create abstract addr"
        );
        addr_local_mem.write(&local.0)?;
        let addr_len = size_of::<u16>() + local.1;
        println!("bind {}, {:?}, {}", server_fd, addr_local_mem.ptr, addr_len);
        let ret = proc.bind(
            server_fd,
            addr_local_mem.ptr as *const libc::sockaddr,
            addr_len as u32,
        )?;
        if ret != 0 {
            let err = (ret * -1) as i32;
            bail!("cannot bind: {} (#{})", nix::errno::from_i32(err), ret);
        }

        std::thread::sleep_ms(1000);

        Ok(Socket { fd: server_fd })
    }

    pub fn connect(&self, anon_remote_id: u64) -> Result<()> {
        println!("connect local socket to: {}", anon_remote_id);
        //try_with!(listen(sock, 1), "cannot listen on from_fd"); // not supported on this transport

        // TODO accept? no. connect.
        // connect
        let remote = try_with!(
            UnixAddr::new_abstract(&anon_remote_id.to_ne_bytes()),
            "cannot create abstract addr"
        );
        println!("remote addr {:?}", remote);
        println!("remote addr {:?}", remote.0.sun_path);
        let uremote = SockAddr::Unix(remote);
        try_with!(
            connect(self.fd, &uremote),
            "cannot connect to client foobar"
        );

        Ok(())
    }

    pub fn connect_remote(
        &self,
        proc: &inject_syscall::Process,
        anon_id_remote: u64,
        addr_remote_mem: &HvMem<libc::sockaddr_un>,
    ) -> Result<()> {
        println!("connect remote socket to: {}", anon_id_remote);
        // connect
        let remote = try_with!(
            UnixAddr::new_abstract(&anon_id_remote.to_ne_bytes()),
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
            let err = (ret * -1) as i32;
            bail!(
                "new_client_remote connect failed: {} (#{})",
                nix::errno::from_i32(err),
                err
            );
        }

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
                    Err(nix::Error::Sys(Errno::EAGAIN)) | Err(nix::Error::Sys(Errno::EINTR)) => {
                        continue
                    }
                    Err(e) => return try_with!(Err(e), "recvmsg failed"),
                    Ok(msg) => {
                        for cmsg in msg.cmsgs() {
                            if let ControlMessageOwned::ScmRights(fds) = cmsg {
                                for fd in fds {
                                    files.push(fd)
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

    /// MT: A single message of this type is received. Example: `[u8; 8]`
    /// CM: Control message (cmsg) space. Example: `[0u8; Tracee::CMSG_SPACE((size_of::<RawFd>() * 2) as u32) as _]`
    pub fn receive_remote<MT: Sized + Copy, CM: Sized + Copy>(
        &self,
        proc: &inject_syscall::Process,
        msg_hdr_mem: &HvMem<libc::msghdr>,
        iov_mem: &HvMem<libc::iovec>,
        iov_buf_mem: &HvMem<MT>,
        cmsg_mem: &HvMem<CM>,
    ) -> Result<(MT, Vec<RawFd>)> {
        println!("receive remote fd: {}", self.fd);

        // init msghdr
        let iov = libc::iovec {
            iov_base: iov_buf_mem.ptr,
            iov_len: size_of::<MT>(),
        };
        iov_mem.write(&iov)?;
        let msg_hdr = libc::msghdr {
            msg_name: 0 as *mut libc::c_void,
            msg_namelen: 0,
            msg_iov: iov_mem.ptr as *mut libc::iovec,
            msg_iovlen: 1,
            msg_control: cmsg_mem.ptr as *mut libc::c_void,
            msg_controllen: size_of::<CM>(),
            msg_flags: 0,
        };
        msg_hdr_mem.write(&msg_hdr)?;

        // recvmsg
        loop {
            println!("revcmsg...");
            let ret = proc.recvmsg(self.fd, msg_hdr_mem.ptr as *mut libc::msghdr, 0)?;
            if ret == 0 {
                bail!("received empty message");
            }
            if ret < 0 {
                let err = (ret * -1) as i32;
                match nix::errno::from_i32(err) {
                    Errno::EAGAIN | Errno::EINTR => continue,
                    e => bail!("recvmsg failed: {} (#{})", e, err),
                }
            }
            println!("done");
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
            println!("cmsghdr: {:?}", *cmsghdr_ptr);
            let cmsghdr: libc::cmsghdr = *cmsghdr_ptr;

            // parse SCM_RIGHTS message
            println!(
                "cmsghdr.cmsg_type {} =? {}",
                cmsghdr.cmsg_type,
                libc::SCM_RIGHTS
            );
            if cmsghdr.cmsg_type != libc::SCM_RIGHTS {
                bail!("cmsghdr not understood");
            }
            println!("cmsghdr.cmsg_len {}", cmsghdr.cmsg_len);

            // iterate over SCM_RIGHTS message data
            let cmsg_data: *mut RawFd = Tracee::CMSG_DATA(cmsghdr_ptr) as *mut RawFd;
            let cmsg_data_len = cmsghdr.cmsg_len - (cmsg_data as usize - cmsghdr_ptr as usize); // cmsg_size - cmsg_hdr_size
            for offset in 0..(cmsg_data_len / size_of::<RawFd>()) {
                result.push(*(cmsg_data.add(offset)));
            }
        }

        Ok((msg_buf, result))
    }
}
