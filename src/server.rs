use anyhow::Result;
use rmcp::{ServiceExt, transport::stdio};

use crate::tools::ObsidianService;

pub async fn serve(service: ObsidianService) -> Result<()> {
    let server = service.serve(stdio()).await?;
    server.waiting().await?;
    Ok(())
}
