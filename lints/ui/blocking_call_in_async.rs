use std::time::Duration;

async fn sleeps() {
    std::thread::sleep(Duration::from_millis(1));
}

async fn reads_file() -> std::io::Result<String> {
    std::fs::read_to_string("Cargo.toml")
}

async fn opens_file() -> std::io::Result<std::fs::File> {
    std::fs::File::open("Cargo.toml")
}

async fn runs_process() -> std::io::Result<std::process::Output> {
    std::process::Command::new("true").output()
}

fn async_block() -> impl std::future::Future<Output = ()> {
    async {
        std::thread::sleep(Duration::from_millis(1));
    }
}

async fn receives(rx: std::sync::mpsc::Receiver<u8>) -> Option<u8> {
    rx.recv().ok()
}

// Not linted: synchronous code may block.
fn sync_sleep() {
    std::thread::sleep(Duration::from_millis(1));
}

// Not linted: the async alternatives.
async fn async_alternatives() -> std::io::Result<String> {
    tokio::time::sleep(Duration::from_millis(1)).await;
    tokio::fs::read_to_string("Cargo.toml").await
}

// Not linted: work moved into a synchronous closure such as `spawn_blocking`.
async fn offloaded() -> std::io::Result<String> {
    let read = || std::fs::read_to_string("Cargo.toml");
    read()
}

fn main() {
    let _ = (sleeps, reads_file, opens_file, runs_process, async_block, receives, sync_sleep, async_alternatives, offloaded);
}
