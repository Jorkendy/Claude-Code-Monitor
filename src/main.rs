use anyhow::Result;
use cc_monitor::cli;
use clap::Parser;

fn main() -> Result<()> {
    cli::Args::parse().run()
}
