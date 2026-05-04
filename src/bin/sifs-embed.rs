use anyhow::Result;
use clap::Parser;
use sifs::{ModelLoadPolicy, ModelOptions, load_model_with_options};

#[derive(Parser)]
#[command(about = "Encode text with the SIFS embedding model and print JSON.")]
struct Args {
    text: String,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    offline: bool,
    #[arg(long = "no-download")]
    no_download: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let policy = if args.offline {
        ModelLoadPolicy::Offline
    } else if args.no_download {
        ModelLoadPolicy::NoDownload
    } else {
        ModelLoadPolicy::AllowDownload
    };
    let model = load_model_with_options(&ModelOptions::new(args.model.as_deref(), policy))?;
    let encoded = model.encode(&[args.text]);
    let row = encoded.row(0).to_vec();
    println!("{}", serde_json::to_string(&row)?);
    Ok(())
}
