use ioutils::shovel::{shovel, FilePair};
use ioutils::tmp;
use libc::SOMAXCONN;
use log::debug;
use nix::sys::socket::{self, listen, AddressFamily, SockAddr, SockFlag, SockType};
use nix::sys::time::{TimeVal, TimeValLike};
use nix::sys::{
    select,
    socket::{accept, bind, VsockAddr},
};
use simple_error::{require_with, try_with};
use std::fs::File;
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::Arc;

use crate::result::Result;
use crate::{interrutable_thread::InterrutableThread, signal_handler};

fn wait_connection(sock: File, should_stop: &Arc<AtomicBool>) -> Result<Option<File>> {
    let mut read_set = select::FdSet::new();

    loop {
        read_set.clear();
        read_set.insert(sock.as_raw_fd());
        let highest = require_with!(read_set.highest(), "cannot get highest index") + 1;
        try_with!(
            select::select(
                highest,
                Some(&mut read_set),
                None,
                None,
                &mut TimeVal::milliseconds(300)
            ),
            "select() failed"
        );
        if read_set.contains(sock.as_raw_fd()) {
            break;
        }
        if should_stop.load(Ordering::Relaxed) {
            return Ok(None);
        }
    }
    let conn_fd = try_with!(accept(sock.as_raw_fd()), "failed to accept connection");
    let conn_file: File = unsafe { File::from_raw_fd(conn_fd.as_raw_fd()) };

    Ok(Some(conn_file))
}

fn pty_serve(vsock: File, should_stop: Arc<AtomicBool>) -> Result<()> {
    debug!("listen for pty vsock connections");
    let vsock_conn = match wait_connection(vsock, &should_stop)? {
        None => return Ok(()),
        Some(c) => c,
    };

    debug!("got pty vsock connections");

    let tempdir = try_with!(tmp::tempdir(), "failed to create tempdir");
    let sockname = tempdir.path().join("sock");
    let unix_fd = try_with!(
        socket::socket(
            AddressFamily::Unix,
            SockType::Stream,
            SockFlag::empty(),
            None
        ),
        "socket failed"
    );
    let unix_sock = unsafe { File::from_raw_fd(unix_fd) };
    let sockaddr = SockAddr::new_unix(&sockname).unwrap();
    try_with!(
        bind(unix_sock.as_raw_fd(), &sockaddr),
        "bind failed {}",
        sockname.display()
    );
    try_with!(
        listen(unix_sock.as_raw_fd(), 10),
        "listen failed {}",
        sockname.display()
    );
    println!(
        "Stage2 ready. Connect to terminal:\nsocat -,raw,echo=0 {}",
        sockname.display()
    );
    let unix_conn = match wait_connection(unix_sock, &should_stop)? {
        // TODO flush any output here in case the user could not connect before the program crashed.
        None => return Ok(()),
        Some(c) => c,
    };

    while shovel(
        &mut [
            FilePair::new(&vsock_conn, &unix_conn),
            FilePair::new(&unix_conn, &vsock_conn),
        ],
        Some(300),
    ) {
        if should_stop.load(Ordering::Relaxed) {
            break;
        }
    }

    Ok(())
}

fn monitor_serve(vsock: File, should_stop: Arc<AtomicBool>) -> Result<()> {
    debug!("listen for monitor vsock connections");
    let vsock_conn = match wait_connection(vsock, &should_stop)? {
        None => return Ok(()),
        Some(c) => c,
    };

    debug!("got monitor vsock connections");

    let stdout: File = unsafe { File::from_raw_fd(libc::STDOUT_FILENO) };

    while shovel(&mut [FilePair::new(&vsock_conn, &stdout)], Some(300)) {
        if should_stop.load(Ordering::Relaxed) {
            break;
        }
    }

    Ok(())
}

fn listen_vsock(port: u32) -> Result<File> {
    let raw_sock = try_with!(
        socket::socket(
            AddressFamily::Vsock,
            SockType::Stream,
            SockFlag::SOCK_CLOEXEC,
            None
        ),
        "cannot create socket"
    );
    let sock = unsafe { File::from_raw_fd(raw_sock) };

    let addr = VsockAddr::new(2, port);

    try_with!(
        socket::bind(sock.as_raw_fd(), &SockAddr::Vsock(addr)),
        "cannot bind vsock({})",
        addr
    );

    try_with!(
        listen(sock.as_raw_fd(), SOMAXCONN as usize),
        "cannot listen on vsock"
    );

    Ok(sock)
}

pub fn pty_thread(result_sender: &SyncSender<()>) -> Result<InterrutableThread<()>> {
    let sock = listen_vsock(9999)?;
    let res = InterrutableThread::spawn(
        "pty-forwarder",
        result_sender,
        move |should_stop: Arc<AtomicBool>| {
            let err = pty_serve(sock, should_stop);
            signal_handler::stop_vmsh();
            err
        },
    );
    Ok(try_with!(res, "failed to spawn thread"))
}

pub fn monitor_thread(result_sender: &SyncSender<()>) -> Result<InterrutableThread<()>> {
    let sock = listen_vsock(9998)?;

    let res = InterrutableThread::spawn(
        "monitor-forwarder",
        result_sender,
        move |should_stop: Arc<AtomicBool>| {
            let err = monitor_serve(sock, should_stop);
            signal_handler::stop_vmsh();
            err
        },
    );
    Ok(try_with!(res, "failed to spawn thread"))
}
