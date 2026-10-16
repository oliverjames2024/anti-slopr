#[tokio::main]
async fn main() {
    if let Err(err) = inventory_sync::refresh("feed.json").await {
        eprintln!("{err}");
    }
}
