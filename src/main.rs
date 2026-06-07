mod cache;
mod cli;
mod joiner;
mod liveness;
mod model;
mod parser;
mod pricing;
mod renderer;
mod scanner;

use anyhow::Result;
use clap::Parser;

fn main() -> Result<()> {
    cli::Args::parse().run()
}
