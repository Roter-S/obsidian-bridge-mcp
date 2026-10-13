#[tokio::main]
async fn main() -> anyhow::Result<()> {
    obsidian_mcp_rs::run().await
}
