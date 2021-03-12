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

/// Reads a .VHS file to JSON
///
/// Each data structure on the file is printed on its own line for easy diffing
/// against other VHS files. Pipe into `jq` to pretty-print.
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

    /// The .VHS file to read
    #[structopt(short, long)]
    input: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::from_args();
    init_logger(args.verbose, args.timestamps)?;
    read_vhs(&args.input)?;
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

fn get_posit<S: Seek>(s: &mut S) -> u32 {
    s.seek(io::SeekFrom::Current(0))
        .expect("Couldn't get current stream position") as u32
}

/// Read (and print) the .VHS file
///
/// VHS files have a few sections:
///
/// 1. A header with some magic bytes, offsets into other sections of the file,
///    flight time of day, etc.
///
/// 2. A list of entities - planes, etc. which move around the world
///
/// 3. A list of "features" which get an initial position and then stay there.
///
/// 4. A lits of position updates for entities and features (each feature has one).
///    Updates don't contain the UID of the entity or feature they apply to.
///    Instead, each entity & feature has a "head" offset that points to their
///    first update, and each update had a "previous" and "next" offset, forming
///    a doubly-linked list of updates for each entity & feature.
///
/// 5. A lists of non-position "events" for entities (switch & DOF changes),
///    similarly chained in doubly-linked lists
///
/// 6. A list of "general" events, split into two parts:
///    - Event "Headers" with most of the data (position, orientation, velocity,
///      scale, flags...)
///    - Event "trailers" sorted chronologically by timestamp with the index
///      of their corresponding header
///
/// 7. Feature events containing a feature index, a timestamp, and a state change
///
/// 8. A set of calligns and team colors.
fn read_vhs(input: &Path) -> Result<()> {
    let mut fh = io::BufReader::new(
        File::open(input).with_context(|| format!("Couldn't open {}", input.display()))?,
    );
    let fh = &mut fh;

    println!("{{");

    let header = read_header(fh)?;
    read_entities(&header, fh)?;
    read_features(&header, fh)?;
    read_position_updates(&header, fh)?;
    read_entity_events(&header, fh)?;
    read_general_events(&header, fh)?;
    read_feature_events(&header, fh)?;
    read_callsigns(&header, fh)?;

    println!("\n}}");
    Ok(())
}

fn read_header(fh: &mut io::BufReader<File>) -> Result<TapeHeader> {
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
    serde_json::to_writer(&io::stdout(), &header)?;
    println!(",");
    Ok(header)
}

fn read_entities(header: &TapeHeader, fh: &mut io::BufReader<File>) -> Result<()> {
    let posit = get_posit(fh);
    ensure!(
        header.entity_offset == posit,
        "Expected entities to start at {}, currently at {}",
        header.entity_offset,
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
        serde_json::to_writer(&io::stdout(), &entity)?;
        println!("{}", if i < header.entity_count - 1 { "," } else { "" });
    }
    println!("],");
    Ok(())
}

fn read_features(header: &TapeHeader, fh: &mut io::BufReader<File>) -> Result<()> {
    let posit = get_posit(fh);
    ensure!(
        header.feature_offset == posit,
        "Expected features to start at {}, currently at {}",
        header.feature_offset,
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
        serde_json::to_writer(&io::stdout(), &feature)?;
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
    Ok(())
}

fn read_position_updates(header: &TapeHeader, fh: &mut io::BufReader<File>) -> Result<()> {
    let posit = get_posit(fh);
    ensure!(
        header.position_offset == posit,
        "Expected position updates to start at {}, currently at {}",
        header.position_offset,
        posit
    );
    ensure!(
        header.position_count >= 0,
        "Negative ({}) position update entry count",
        header.entity_count
    );
    println!("\"position updates\": [");
    for i in 0..header.position_count {
        let entry = TimelineEntry::read(fh)?;
        serde_json::to_writer(&io::stdout(), &entry)?;
        println!(
            "{}",
            if i < header.position_count - 1 {
                ","
            } else {
                ""
            }
        );
    }
    println!("],");
    Ok(())
}

fn read_entity_events(header: &TapeHeader, fh: &mut io::BufReader<File>) -> Result<()> {
    let posit = get_posit(fh);
    ensure!(
        header.entity_event_offset == posit,
        "Expected entity events to start at {}, currently at {}",
        header.entity_event_offset,
        posit
    );
    ensure!(
        header.entity_event_count >= 0,
        "Negative ({}) timeline entry count",
        header.entity_event_count
    );
    println!("\"entity events\": [");
    for i in 0..header.entity_event_count {
        let entry = TimelineEntry::read(fh)?;

        serde_json::to_writer(&io::stdout(), &entry)?;
        println!(
            "{}",
            if i < header.entity_event_count - 1 {
                ","
            } else {
                ""
            }
        );
    }
    println!("],");
    Ok(())
}

fn read_general_events(header: &TapeHeader, fh: &mut io::BufReader<File>) -> Result<()> {
    let mut posit = get_posit(fh);
    ensure!(
        header.general_event_offset == posit,
        "Expected general event headers to start at {}, currently at {}",
        header.general_event_offset,
        posit
    );
    ensure!(
        header.general_event_count >= 0,
        "Negative ({}) timeline entry count",
        header.general_event_count
    );
    println!("\"general event headers\": [");
    for i in 0..header.general_event_count {
        let entry = GeneralEventHeader::read(fh)?;

        serde_json::to_writer(&io::stdout(), &entry)?;
        println!(
            "{}",
            if i < header.general_event_count - 1 {
                ","
            } else {
                ""
            }
        );
    }
    println!("],");

    posit = get_posit(fh);
    ensure!(
        header.general_event_trailer_offset == posit,
        "Expected general event trailers to start at {}, currently at {}",
        header.general_event_trailer_offset,
        posit
    );
    println!("\"general event trailers\": [");
    for i in 0..header.general_event_count {
        let entry = GeneralEventTrailer::read(fh)?;

        serde_json::to_writer(&io::stdout(), &entry)?;
        println!(
            "{}",
            if i < header.general_event_count - 1 {
                ","
            } else {
                ""
            }
        );
    }
    println!("],");
    Ok(())
}

fn read_feature_events(header: &TapeHeader, fh: &mut io::BufReader<File>) -> Result<()> {
    let posit = get_posit(fh);
    ensure!(
        header.feature_event_offset == posit,
        "Expected feature events to start at {}, currently at {}",
        header.feature_event_offset,
        posit
    );
    ensure!(
        header.feature_event_count >= 0,
        "Negative ({}) timeline entry count",
        header.feature_event_count
    );
    println!("\"feature events\": [");
    for i in 0..header.feature_event_count {
        let entry = FeatureEvent::read(fh)?;

        serde_json::to_writer(&io::stdout(), &entry)?;
        println!(
            "{}",
            if i < header.feature_event_count - 1 {
                ","
            } else {
                ""
            }
        );
    }
    println!("],");
    Ok(())
}

fn read_callsigns(header: &TapeHeader, fh: &mut io::BufReader<File>) -> Result<()> {
    let posit = get_posit(fh);
    ensure!(
        header.text_event_offset == posit,
        "Expected text events to start at {}, currently at {}",
        header.text_event_offset,
        posit
    );

    // For reasons I don't understand, the callsign count is saved
    // in four bytes preceding the block instead of as `text_event_count`
    // in the file header.
    let callsign_count = read_primitives::read_i32(fh)?;
    ensure!(
        callsign_count >= 0,
        "Negative ({}) timeline entry count",
        callsign_count
    );
    println!("\"callsigns\": [");
    for i in 0..callsign_count {
        let callsign = CallsignRecord::read(fh)?;

        serde_json::to_writer(&io::stdout(), &callsign)?;
        println!("{}", if i < callsign_count - 1 { "," } else { "" });
    }
    println!("]");
    Ok(())
}
