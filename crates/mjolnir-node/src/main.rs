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
    /// Enter a mesh room. Creates a new room or joins an existing one via ticket.
    /// Every peer is equal — any peer can share their ticket for others to join.
    Room {
        /// Room name
        #[arg(default_value = "default")]
        name: String,

        /// Join ticket from an existing peer (name@base32payload).
        /// If omitted, creates a new room.
        #[arg(short, long)]
        ticket: Option<String>,
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
        Command::Room { name, ticket } => {
            let node = mesh::MeshNode::spawn().await?;
            info!("endpoint id: {}", node.id());

            // Unified entry: with ticket = join, without = create
            let our_ticket = node.enter_room(&name, ticket.as_deref()).await?;
            println!("\n  Invite ticket: {our_ticket}\n");

            // Run room actor until ctrl-c
            tokio::select! {
                result = node.run_room() => {
                    if let Err(e) = result {
                        tracing::error!("room error: {e}");
                    }
                }
                _ = tokio::signal::ctrl_c() => {}
            }
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
