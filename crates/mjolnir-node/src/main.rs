use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::info;
use tracing_subscriber::EnvFilter;

mod mesh;
mod room;
mod ticket;

#[derive(Parser)]
#[command(name = "mjolnir-mesh", about = "P2P audio mesh over iroh + MoQ")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start a new mesh room and begin broadcasting audio
    Host {
        /// Room name (included in the join ticket)
        #[arg(short, long, default_value = "default")]
        name: String,
    },

    /// Join an existing mesh room by ticket
    Join {
        /// Ticket string from the host (name@base32addr)
        ticket: String,
    },

    /// Print this node's iroh EndpointId
    Id,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Host { name } => {
            let node = mesh::MeshNode::spawn().await?;
            info!("endpoint id: {}", node.id());

            let ticket = node.host_room(&name).await?;
            println!("\n  Join ticket: {ticket}\n");

            // Block until ctrl-c
            tokio::signal::ctrl_c().await?;
            node.shutdown().await;
        }

        Command::Join { ticket } => {
            let node = mesh::MeshNode::spawn().await?;
            info!("endpoint id: {}", node.id());

            node.join_room(&ticket).await?;

            tokio::signal::ctrl_c().await?;
            node.shutdown().await;
        }

        Command::Id => {
            let node = mesh::MeshNode::spawn().await?;
            println!("{}", node.id());
            node.shutdown().await;
        }
    }

    Ok(())
}
