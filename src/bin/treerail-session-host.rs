#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let endpoint = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("A private local endpoint name is required"))?;
    if endpoint == "--managed-child" {
        return treerail::host::entry::managed_child(args.map(Into::into));
    }
    treerail::host::runtime::serve(&endpoint).await
}
