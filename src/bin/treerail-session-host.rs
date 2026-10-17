#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let endpoint = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("A private local endpoint name is required"))?;
    if endpoint == "--managed-child" {
        // The host assigns this process to its execution group before releasing
        // this gate. No real agent or tools may start outside that group.
        let mut line = String::new();
        anyhow::ensure!(
            std::io::stdin().read_line(&mut line)? > 0,
            "Managed launch gate closed"
        );
        anyhow::ensure!(
            line.trim() == "treerail-start",
            "Invalid managed launch gate"
        );
        let program = args
            .next()
            .ok_or_else(|| anyhow::anyhow!("Managed child program is required"))?;
        let status = std::process::Command::new(program).args(args).status()?;
        std::process::exit(status.code().unwrap_or(1));
    }
    treerail::host::runtime::serve(&endpoint).await
}
