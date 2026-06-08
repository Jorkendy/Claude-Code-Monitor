use anyhow::Result;
use tokenscope::cli;
use clap::Parser;

fn main() -> Result<()> {
    cli::Args::parse().run()
}
