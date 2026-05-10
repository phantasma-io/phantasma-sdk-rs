use phantasma_sdk::{PhantasmaRpc, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let endpoint = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("PHANTASMA_RPC_URL").ok())
        .unwrap_or_else(|| "http://localhost:5172/rpc".to_string());

    let rpc = PhantasmaRpc::new(endpoint);
    let version = rpc.get_version().await?;
    println!("version={} commit={}", version.version, version.commit);
    Ok(())
}
