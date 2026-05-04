use anyhow::Result;
use clap::Parser;
use sifs::load_model;

#[derive(Parser)]
#[command(about = "Encode text with the SIFS embedding model and print JSON.")]
struct Args {
    text: String,
    #[arg(long)]
    model: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let model = load_model(args.model.as_deref())?;
    let encoded = model.encode(&[args.text]);
    let row = encoded.row(0).to_vec();
    println!("{}", serde_json::to_string(&row)?);
    Ok(())
}
