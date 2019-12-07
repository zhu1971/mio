#![cfg(unix)]
#[macro_use]
mod util;

use mio::net::UnixListener;
use mio::{Interest, Token};
use std::io::{self, Read};
use std::os::unix::net;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::thread;
use tempdir::TempDir;
use util::{
    assert_send, assert_sync, assert_would_block, expect_events, expect_no_events, init_with_poll,
    ExpectEvent,
};

const DEFAULT_BUF_SIZE: usize = 64;
const TOKEN_1: Token = Token(0);

#[test]
fn unix_listener_send_and_sync() {
    assert_send::<UnixListener>();
    assert_sync::<UnixListener>();
}

#[test]
fn unix_listener_smoke() {
    #[allow(clippy::redundant_closure)]
    smoke_test(|path| UnixListener::bind(path));
}

#[test]
fn unix_listener_from_std() {
    smoke_test(|path| {
        let listener = net::UnixListener::bind(path).unwrap();
        // `std::os::unix::net::UnixStream`s are blocking by default, so make sure
        // it is in non-blocking mode before wrapping in a Mio equivalent.
        listener.set_nonblocking(true).unwrap();
        Ok(UnixListener::from_std(listener))
    })
}

#[test]
fn unix_listener_local_addr() {
    let (mut poll, mut events) = init_with_poll();
    let barrier = Arc::new(Barrier::new(2));
    let dir = TempDir::new("unix_listener").unwrap();
    let path = dir.path().join("any");

    let mut listener = UnixListener::bind(&path).unwrap();
    poll.registry()
        .register(
            &mut listener,
            TOKEN_1,
            Interest::WRITABLE.add(Interest::READABLE),
        )
        .unwrap();

    let handle = open_connections(path.clone(), 1, barrier.clone());
    expect_events(
        &mut poll,
        &mut events,
        vec![ExpectEvent::new(TOKEN_1, Interest::READABLE)],
    );

    let (stream, expected_addr) = listener.accept().unwrap();
    assert_eq!(stream.local_addr().unwrap().as_pathname().unwrap(), &path);
    assert!(expected_addr.as_pathname().is_none());

    barrier.wait();
    handle.join().unwrap();
}

#[test]
fn unix_listener_register() {
    let (mut poll, mut events) = init_with_poll();
    let dir = TempDir::new("unix_listener").unwrap();

    let mut listener = UnixListener::bind(dir.path().join("any")).unwrap();
    poll.registry()
        .register(&mut listener, TOKEN_1, Interest::READABLE)
        .unwrap();
    expect_no_events(&mut poll, &mut events)
}

#[test]
fn unix_listener_reregister() {
    let (mut poll, mut events) = init_with_poll();
    let barrier = Arc::new(Barrier::new(2));
    let dir = TempDir::new("unix_listener").unwrap();
    let path = dir.path().join("any");

    let mut listener = UnixListener::bind(&path).unwrap();
    poll.registry()
        .register(&mut listener, TOKEN_1, Interest::WRITABLE)
        .unwrap();

    let handle = open_connections(path, 1, barrier.clone());
    expect_no_events(&mut poll, &mut events);

    poll.registry()
        .reregister(&mut listener, TOKEN_1, Interest::READABLE)
        .unwrap();
    expect_events(
        &mut poll,
        &mut events,
        vec![ExpectEvent::new(TOKEN_1, Interest::READABLE)],
    );

    barrier.wait();
    handle.join().unwrap();
}

#[test]
fn unix_listener_deregister() {
    let (mut poll, mut events) = init_with_poll();
    let barrier = Arc::new(Barrier::new(2));
    let dir = TempDir::new("unix_listener").unwrap();
    let path = dir.path().join("any");

    let mut listener = UnixListener::bind(&path).unwrap();
    poll.registry()
        .register(&mut listener, TOKEN_1, Interest::READABLE)
        .unwrap();

    let handle = open_connections(path, 1, barrier.clone());

    poll.registry().deregister(&mut listener).unwrap();
    expect_no_events(&mut poll, &mut events);

    barrier.wait();
    handle.join().unwrap();
}

fn smoke_test<F>(new_listener: F)
where
    F: FnOnce(&Path) -> io::Result<UnixListener>,
{
    let (mut poll, mut events) = init_with_poll();
    let barrier = Arc::new(Barrier::new(2));
    let dir = TempDir::new("unix_listener").unwrap();
    let path = dir.path().join("any");

    let mut listener = new_listener(&path).unwrap();
    poll.registry()
        .register(
            &mut listener,
            TOKEN_1,
            Interest::WRITABLE.add(Interest::READABLE),
        )
        .unwrap();
    expect_no_events(&mut poll, &mut events);

    let handle = open_connections(path, 1, barrier.clone());
    expect_events(
        &mut poll,
        &mut events,
        vec![ExpectEvent::new(TOKEN_1, Interest::READABLE)],
    );

    let (mut stream, _) = listener.accept().unwrap();

    let mut buf = [0; DEFAULT_BUF_SIZE];
    assert_would_block(stream.read(&mut buf));

    assert_would_block(listener.accept());
    assert!(listener.take_error().unwrap().is_none());

    barrier.wait();
    handle.join().unwrap();
}

fn open_connections(
    path: PathBuf,
    n_connections: usize,
    barrier: Arc<Barrier>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        for _ in 0..n_connections {
            let conn = net::UnixStream::connect(path.clone()).unwrap();
            barrier.wait();
            drop(conn);
        }
    })
}