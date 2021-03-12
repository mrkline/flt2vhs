use std::fs::File;
use std::io;
use std::io::prelude::*;
use std::path::{Path, PathBuf};

use anyhow::*;
use log::*;
use simplelog::*;
use structopt::StructOpt;

mod flt;
mod read_primitives;

/// Converts a .FLT file to .VHS
#[derive(Debug, StructOpt)]
#[structopt(verbatim_doc_comment)]
struct Args {
    /// Verbosity (-v, -vv, -vvv, etc.)
    #[structopt(short, long, parse(from_occurrences))]
    verbose: u8,

    /// Prepend ISO-8601 timestamps to all trace messages
    /// (from --verbose). Useful for benchmarking.
    #[structopt(short, long, verbatim_doc_comment)]
    timestamps: bool,

    /// The .FLT file to read
    #[structopt(short, long)]
    input: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::from_args();
    init_logger(args.verbose, args.timestamps)?;
    let mapping = open_file(&args.input)?;

    let parsed_flight = flt::Flight::parse(&*mapping);
    if parsed_flight.corrupted {
        warn!("Flight file is corrupted! Doing what we can with what we have...");
    }

    if parsed_flight.corrupted {
        warn!("Converted corrupted FLT file, resulting VHS may be incomplete");
        std::process::exit(2); // Use a different error code than normal failure
    } else {
        Ok(())
    }
}

/// Set up simplelog to spit messages to stderr.
fn init_logger(verbosity: u8, timestamps: bool) -> Result<()> {
    let mut builder = ConfigBuilder::new();
    // Shut a bunch of stuff off - we're just spitting to stderr.
    builder.set_location_level(LevelFilter::Trace);
    builder.set_target_level(LevelFilter::Off);
    builder.set_thread_level(LevelFilter::Off);
    if timestamps {
        builder.set_time_format_str("%+");
        builder.set_time_level(LevelFilter::Error);
    } else {
        builder.set_time_level(LevelFilter::Off);
    }

    let level = match verbosity {
        0 => LevelFilter::Warn,
        1 => LevelFilter::Info,
        2 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    };

    let config = builder.build();

    if cfg!(test) {
        TestLogger::init(level, config).context("Couldn't init test logger")
    } else {
        TermLogger::init(level, config.clone(), TerminalMode::Stderr)
            .or_else(|_| SimpleLogger::init(level, config))
            .context("Couldn't init logger")
    }
}

fn open_file(f: &Path) -> Result<memmap::Mmap> {
    let fh = File::open(f).with_context(|| format!("Couldn't open {}", f.display()))?;
    let mapping = unsafe { memmap::Mmap::map(&fh)? };
    // An madvise(MADV_SEQUENTIAL) might be nice here
    // since we're not seeking around.
    Ok(mapping)
}
