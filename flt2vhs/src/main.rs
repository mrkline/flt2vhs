use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::*;
use humansize::{file_size_opts as Sizes, FileSize};
use log::*;
use structopt::StructOpt;

mod flt;
mod primitives;
mod vhs;

/// Converts a FLT file to VHS
#[derive(Debug, StructOpt)]
#[structopt(verbatim_doc_comment)]
struct Args {
    /// Verbosity (-v, -vv, -vvv, etc.)
    #[structopt(short, long, parse(from_occurrences))]
    verbose: u8,

    #[structopt(short, long, case_insensitive = true, default_value = "auto")]
    #[structopt(name = "always/auto/never")]
    color: logsetup::Color,

    /// Prepend ISO-8601 timestamps to all messages
    /// (from --verbose). Useful for benchmarking.
    #[structopt(short, long, verbatim_doc_comment)]
    timestamps: bool,

    /// Delete the FLT file after successful conversion to VHS
    #[structopt(short, long)]
    delete: bool,

    /// Overwrite output files, even if input was corrupted.
    #[structopt(short, long)]
    force: bool,

    /// The FLT file to read
    #[structopt(name = "input.flt")]
    inputs: Vec<PathBuf>,
}

pub fn print_timing(msg: &str, start: &Instant) {
    info!("{} took {:.3}s", msg, start.elapsed().as_secs_f32());
}

fn main() {
    run().unwrap_or_else(|e| {
        error!("{:?}", e);
        std::process::exit(1);
    });
}

fn run() -> Result<()> {
    let start_time = Instant::now();

    let args = Args::from_args();
    logsetup::init_logger(args.verbose, args.timestamps, args.color);

    let parse_start = Instant::now();

    let mut flights: Vec<_> = args
        .inputs
        .iter()
        .map(|input| {
            info!("Parsing {}", input.display());

            let mapping = open_flt(&input)?;
            let parsed_flight = flt::Flight::parse(&*mapping);
            drop(mapping);

            Ok(parsed_flight)
        })
        .collect::<Result<Vec<_>>>()?;

    print_timing(
        &format!("Parsing {} FLT files", args.inputs.len()),
        &parse_start,
    );

    let mut starting_index = 0;
    let mut next_index = 1;

    while starting_index < flights.len() {
        // All hail the borrow checker
        let (left, right) = flights.split_at_mut(next_index);
        let starting = &mut left[starting_index];

        if right.is_empty()
            || !starting.merge(
                &right[0],
                &args.inputs[starting_index],
                &args.inputs[next_index],
            )
        {
            write_flight(&args.inputs[starting_index..next_index], starting, &args)?;
            starting_index = next_index;
            next_index = starting_index + 1;
        } else {
            next_index += 1;
        }
    }

    info!(
        "All files converted in {:.3}s",
        start_time.elapsed().as_secs_f32(),
    );
    Ok(())
}

fn output_name(input: &Path) -> Result<PathBuf> {
    // Path::with_extension just replaces the last one.
    // Replace ALL THE EXTENISONS!
    let name = input
        .file_name()
        .ok_or_else(|| anyhow!("{} isn't a file name!", input.display()))?;
    let as_str = name
        .to_str()
        .ok_or_else(|| anyhow!("Can't remove the extension from {}", input.display()))?;
    Ok(PathBuf::from(
        as_str.split('.').next().unwrap().to_owned() + ".vhs",
    ))
}

fn open_flt(f: &Path) -> Result<memmap::Mmap> {
    let fh = File::open(f).with_context(|| format!("Couldn't open {}", f.display()))?;
    let mapping = unsafe { memmap::Mmap::map(&fh) }
        .with_context(|| format!("Couldn't memory map {}", f.display()))?;
    // An madvise(MADV_SEQUENTIAL) might be nice here
    // since we're not seeking around.
    Ok(mapping)
}

fn open_vhs(to: &Path) -> Result<File> {
    let fh = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(to)
        .with_context(|| format!("Couldn't open {} to write", to.display()))?;
    Ok(fh)
}

fn write_flight(inputs: &[PathBuf], flight: &flt::Flight, args: &Args) -> Result<()> {
    let output = output_name(&inputs[0])?;

    let flt_size = inputs
        .iter()
        .map(fs::metadata)
        .fold(Ok(0), |acc, meta| -> Result<u64, Error> {
            acc.and_then(|sum| Ok(sum + meta?.len()))
        })?;

    let mut size_options = Sizes::CONVENTIONAL;
    size_options.space = false;

    info!(
        "Converting {} ({}) to {}",
        inputs
            .iter()
            .map(|i| i.to_string_lossy())
            .collect::<Vec<_>>()
            .join(", "),
        flt_size.file_size(&size_options).unwrap(),
        output.display()
    );

    if flight.corrupted {
        warn!("Flight file is corrupted! Doing what we can with what we have...");
    }
    if !args.force && flight.corrupted {
        if inputs[0] == output {
            bail!(
                "{} looks like a VHS file! Quitting before we overwrite it",
                inputs[0].display()
            );
        } else if output.exists() {
            bail!(
                "Refusing to overwrite {} with a corrupted recording without --force",
                output.display()
            );
        }
    }

    let write_start = Instant::now();
    let vhs = open_vhs(&output)?;
    let vhs_size = vhs::write(&flight, vhs)?;
    print_timing(
        &format!(
            "{} ({}) write",
            output.display(),
            vhs_size.file_size(&size_options).unwrap(),
        ),
        &write_start,
    );

    if flight.corrupted {
        warn!("Converted corrupted FLT file, resulting VHS may be incomplete");
        std::process::exit(2); // Use a different error code than normal failure
    } else {
        if args.delete {
            for input in inputs {
                debug!("Deleting {} after its conversion", input.display());
                fs::remove_file(&input)
                    .with_context(|| format!("Couldn't remove {}", input.display()))?;
            }
        }
        Ok(())
    }
}
