use std::fs::File;
use std::io;
use std::io::prelude::*;
use std::path::PathBuf;

use anyhow::*;
use log::*;
use structopt::StructOpt;

mod acmitape;
mod read_primitives;

use crate::acmitape::*;

/// Reads a VHS file to JSON
///
/// Each data structure on the file is printed on its own line for easy diffing
/// against other VHS files. Pipe into `jq` to pretty-print.
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

    /// The VHS file to read
    #[structopt(name = "input.vhs")]
    input: Option<PathBuf>,
}

fn main() {
    run().unwrap_or_else(|e| {
        error!("{:?}", e);
        std::process::exit(1);
    });
}

fn run() -> Result<()> {
    let args = Args::from_args();
    logsetup::init_logger(args.verbose, args.timestamps, args.color);

    let stdin = io::stdin();

    let input: Box<dyn io::Read> = match args.input {
        None => Box::new(stdin.lock()),
        Some(file) => {
            if file.to_string_lossy() == "-" {
                Box::new(stdin.lock())
            } else {
                Box::new(
                    File::open(&file)
                        .with_context(|| format!("Couldn't open {}", file.display()))?,
                )
            }
        }
    };
    let r = io::BufReader::new(input);

    read_vhs(r)?;
    Ok(())
}

struct CountedRead<R> {
    inner: R,
    posit: u32, // Welcome to 1998, where files are always < 4 GB.
}

impl<R: Read> CountedRead<R> {
    fn new(inner: R) -> Self {
        Self { inner, posit: 0 }
    }

    fn get_posit(&self) -> u32 {
        self.posit
    }
}

impl<R: Read> Read for CountedRead<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let res = self.inner.read(buf);
        if let Ok(count) = res {
            self.posit += count as u32;
        }
        res
    }
}

/// Read (and print) the VHS file
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
fn read_vhs<R: Read>(r: R) -> Result<()> {
    let mut counted = CountedRead::new(r);
    let counted = &mut counted;

    println!("{{");

    let header = read_header(counted)?;
    read_entities(&header, counted)?;
    read_features(&header, counted)?;
    read_position_updates(&header, counted)?;
    read_entity_events(&header, counted)?;
    read_general_events(&header, counted)?;
    read_feature_events(&header, counted)?;
    read_callsigns(&header, counted)?;

    println!("\n}}");
    Ok(())
}

fn read_header<R: Read>(r: &mut CountedRead<R>) -> Result<TapeHeader> {
    let header = TapeHeader::read(r)?;
    if &header.file_id != b"EPAT" {
        warn!(
            "Expected magic bytes 'EPAT', got {:?} ({})",
            &header.file_id,
            String::from_utf8_lossy(&header.file_id)
        );
    }

    print!("\"header\": ");
    serde_json::to_writer(&io::stdout(), &header)?;
    println!(",");
    Ok(header)
}

fn read_entities<R: Read>(header: &TapeHeader, r: &mut CountedRead<R>) -> Result<()> {
    let posit = r.get_posit();
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
        let entity = Entity::read(r)?;
        serde_json::to_writer(&io::stdout(), &entity)?;
        println!("{}", if i < header.entity_count - 1 { "," } else { "" });
    }
    println!("],");
    Ok(())
}

fn read_features<R: Read>(header: &TapeHeader, r: &mut CountedRead<R>) -> Result<()> {
    let posit = r.get_posit();
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
        let feature = Entity::read(r)?;
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

fn read_position_updates<R: Read>(header: &TapeHeader, r: &mut CountedRead<R>) -> Result<()> {
    let posit = r.get_posit();
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
        let entry = TimelineEntry::read(r)?;
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

fn read_entity_events<R: Read>(header: &TapeHeader, r: &mut CountedRead<R>) -> Result<()> {
    let posit = r.get_posit();
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
        let entry = TimelineEntry::read(r)?;

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

fn read_general_events<R: Read>(header: &TapeHeader, r: &mut CountedRead<R>) -> Result<()> {
    let mut posit = r.get_posit();
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
        let entry = GeneralEventHeader::read(r)?;

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

    posit = r.get_posit();
    ensure!(
        header.general_event_trailer_offset == posit,
        "Expected general event trailers to start at {}, currently at {}",
        header.general_event_trailer_offset,
        posit
    );
    println!("\"general event trailers\": [");
    for i in 0..header.general_event_count {
        let entry = GeneralEventTrailer::read(r)?;

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

fn read_feature_events<R: Read>(header: &TapeHeader, r: &mut CountedRead<R>) -> Result<()> {
    let posit = r.get_posit();
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
        let entry = FeatureEvent::read(r)?;

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

fn read_callsigns<R: Read>(header: &TapeHeader, r: &mut CountedRead<R>) -> Result<()> {
    let posit = r.get_posit();
    ensure!(
        header.text_event_offset == posit,
        "Expected text events to start at {}, currently at {}",
        header.text_event_offset,
        posit
    );

    // For reasons I don't understand, the callsign count is saved
    // in four bytes preceding the block instead of as `text_event_count`
    // in the file header.
    let callsign_count = read_primitives::read_i32(r)?;
    ensure!(
        callsign_count >= 0,
        "Negative ({}) timeline entry count",
        callsign_count
    );
    println!("\"callsigns\": [");
    for i in 0..callsign_count {
        let callsign = CallsignRecord::read(r)?;

        serde_json::to_writer(&io::stdout(), &callsign)?;
        println!("{}", if i < callsign_count - 1 { "," } else { "" });
    }
    println!("]");
    Ok(())
}
