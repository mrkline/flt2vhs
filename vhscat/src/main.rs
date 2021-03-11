use std::fs::File;
use std::io;
use std::io::prelude::*;
use std::path::{Path, PathBuf};

use anyhow::*;
use log::*;
use simplelog::*;
use structopt::StructOpt;

mod acmitape;
mod read_primitives;

use crate::acmitape::*;

#[derive(Debug, StructOpt)]
#[structopt(verbatim_doc_comment)]
struct Args {
    /// Verbosity (-v, -vv, -vvv, etc.)
    #[structopt(short, long, parse(from_occurrences))]
    verbose: u8,

    /// Prepend ISO-8601 timestamps to all trace messages (from --verbose).
    /// Useful for benchmarking.
    #[structopt(short, long)]
    timestamps: bool,

    #[structopt(short, long)]
    input: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::from_args();
    init_logger(args.verbose, args.timestamps)?;
    read_flt(&args.input)?;

    Ok(())
}

fn get_posit<S: Seek>(s: &mut S) -> u32 {
    s.seek(io::SeekFrom::Current(0))
        .expect("Couldn't get current stream posit") as u32
}

fn read_flt(input: &Path) -> Result<()> {
    let mut fh = io::BufReader::new(
        File::open(input).with_context(|| format!("Couldn't open {}", input.display()))?,
    );
    let fh = &mut fh;
    let out = io::stdout();

    println!("{{");

    let header = TapeHeader::read(fh)?;
    if &header.file_id != b"EPAT" {
        warn!(
            "Expected magic bytes 'EPAT', got {:?} ({})",
            &header.file_id,
            String::from_utf8_lossy(&header.file_id)
        );
    }

    let actual_size = fh.get_ref().metadata()?.len();
    ensure!(
        header.file_size as u64 <= actual_size,
        "Given size ({}) > actual size ({})",
        header.file_size,
        actual_size
    );

    print!("\"header\": ");
    serde_json::to_writer(&out, &header)?;
    println!(",");

    let mut posit = get_posit(fh);
    ensure!(
        header.entity_block_offset == posit,
        "Expected entities to start at {}, currently at {}",
        header.entity_block_offset,
        posit
    );
    ensure!(
        header.entity_count >= 0,
        "Negative ({}) entity count",
        header.entity_count
    );
    println!("\"entities\": [");
    for i in 0..header.entity_count {
        let entity = Entity::read(fh)?;
        serde_json::to_writer(&out, &entity)?;
        println!("{}", if i < header.entity_count - 1 { "," } else { "" });
    }
    println!("],");

    posit = get_posit(fh);
    ensure!(
        header.feature_block_offset == posit,
        "Expected features to start at {}, currently at {}",
        header.feature_block_offset,
        posit
    );
    ensure!(
        header.feature_count >= 0,
        "Negative ({}) feature count",
        header.entity_count
    );
    println!("\"features\": [");
    for i in 0..header.feature_count {
        let feature = Entity::read(fh)?;
        serde_json::to_writer(&out, &feature)?;
        println!(
            "{}",
            if i < header.feature_count - 1 {
                ","
            } else {
                ""
            }
        );
    }
    println!("],");

    posit = get_posit(fh);
    ensure!(
        header.timeline_block_offset == posit,
        "Expected timeline to start at {}, currently at {}",
        header.timeline_block_offset,
        posit
    );
    ensure!(
        header.timeline_count >= 0,
        "Negative ({}) timeline entry count",
        header.entity_count
    );
    println!("\"timeline\": [");
    for i in 0..header.timeline_count {
        let entry = TimelineEntry::read(fh)?;
        serde_json::to_writer(&out, &entry)?;
        println!(
            "{}",
            if i < header.timeline_count - 1 {
                ","
            } else {
                ""
            }
        );
    }
    println!("]");

    println!("\n}}");
    Ok(())
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
