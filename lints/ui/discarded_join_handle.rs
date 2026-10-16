#![allow(dead_code, clippy::let_underscore_future)]

use std::time::Duration;

async fn work() -> u32 {
    tokio::time::sleep(Duration::from_millis(1)).await;
    1
}

async fn tokio_tasks() {
    tokio::spawn(work());

    tokio::task::spawn(async {
        work().await;
    });

    let _ = tokio::spawn(work());

    _ = tokio::spawn(work());

    tokio::task::spawn_blocking(|| 1);

    let runtime_handle = tokio::runtime::Handle::current();
    runtime_handle.spawn(work());
}

fn std_threads() {
    std::thread::spawn(|| 1);

    let _ = std::thread::spawn(|| 1);

    std::thread::Builder::new().name("worker".into()).spawn(|| 1).unwrap();
}

// Not linted: the handle is kept and awaited or joined.
async fn kept() -> Result<u32, tokio::task::JoinError> {
    let handle = tokio::spawn(work());
    let mut set = tokio::task::JoinSet::new();
    set.spawn(work());
    while set.join_next().await.is_some() {}
    handle.await
}

// Not linted: the thread is joined.
fn joined() -> std::thread::Result<u32> {
    let thread = std::thread::spawn(|| 1);
    thread.join()
}

// Not linted: an explicit `drop(..)` documents intentional detaching.
fn detached() {
    let reaper = std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_secs(60));
    });
    // The reaper lives for the whole process.
    drop(reaper);
}

// Not linted: a named binding keeps the handle alive.
fn named() {
    let _handle = std::thread::spawn(|| 1);
}

// Not linted: scoped threads are joined at the end of the scope.
fn scoped() {
    let mut value = 0;
    std::thread::scope(|scope| {
        scope.spawn(|| value += 1);
    });
    let _ = value;
}

// Not linted: the `JoinSet` owns the task.
async fn join_set() {
    let mut set = tokio::task::JoinSet::new();
    set.spawn(work());
    set.join_all().await;
}

// Not linted: the abort handle is dropped, the task is still joined.
async fn abort_handle() {
    let handle = tokio::spawn(work());
    let _ = handle.abort_handle();
    let _ = handle.await;
}

// Not linted: spawned inside a macro expansion.
macro_rules! spawn_in_macro {
    () => {
        tokio::spawn(work());
    };
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    spawn_in_macro!();
    tokio_tasks().await;
    std_threads();
}
