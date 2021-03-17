use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::*;
use log::*;
use simplelog::*;
use structopt::StructOpt;

mod flt;
mod read_primitives;
mod vhs;
mod write_primitives;

/// Converts a FLT file to VHS
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

    /// The VHS file to write. Defaults to <input>.vhs
    #[structopt(short, long, name = "VHS file")]
    output: Option<PathBuf>,

    /// The FLT file to read
    #[structopt(name = "input.flt")]
    input: PathBuf,
}

pub fn print_timing(msg: &str, start: &Instant) {
    info!("{} took {:2.4}s", msg, start.elapsed().as_secs_f32());
}

fn main() -> Result<()> {
    let start_time = Instant::now();

    let args = Args::from_args();
    init_logger(args.verbose, args.timestamps)?;

    let default_output = Path::new(args.input.file_name().unwrap()).with_extension("vhs");
    let output = args.output.unwrap_or(default_output);
    info!(
        "Converting {} to {}",
        args.input.display(),
        output.display()
    );

    let mapping = open_flt(&args.input)?;
    let parse_start = Instant::now();
    let parsed_flight = flt::Flight::parse(&*mapping);
    print_timing("FLT parse", &parse_start);
    if parsed_flight.corrupted {
        warn!("Flight file is corrupted! Doing what we can with what we have...");
    }

    let write_start = Instant::now();
    let vhs = open_vhs(&output)?;
    vhs::write(&parsed_flight, vhs)?;
    print_timing("VHS write", &write_start);

    print_timing("Entire operation", &start_time);
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

fn open_flt(f: &Path) -> Result<memmap::Mmap> {
    let fh = File::open(f).with_context(|| format!("Couldn't open {}", f.display()))?;
    let mapping = unsafe { memmap::Mmap::map(&fh)? };
    // An madvise(MADV_SEQUENTIAL) might be nice here
    // since we're not seeking around.
    Ok(mapping)
}

fn open_vhs(to: &Path) -> Result<File> {
    let fh =
        File::create(to).with_context(|| format!("Couldn't open {} to write", to.display()))?;
    Ok(fh)
}
