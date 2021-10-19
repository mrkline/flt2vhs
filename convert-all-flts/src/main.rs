use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::*;
use chrono::prelude::*;
use log::*;
use structopt::StructOpt;

/// Converts all FLT files in the directory to VHS,
/// then waits for BMS to make more.
#[derive(Debug, StructOpt)]
#[structopt(verbatim_doc_comment)]
struct Args {
    /// Verbosity (-v, -vv, -vvv, etc.). Defaults to `-v`
    #[structopt(short, long, parse(from_occurrences))]
    verbose: u8,

    #[structopt(short, long, case_insensitive = true, default_value = "auto")]
    #[structopt(name = "always/auto/never")]
    color: logsetup::Color,

    /// Prepend ISO-8601 timestamps to all messages
    /// (from --verbose). Useful for benchmarking.
    #[structopt(short, long, verbatim_doc_comment)]
    timestamps: bool,

    /// Switch to this working directory (e.g., BMS/User/Acmi)
    /// before doing anything else.
    #[structopt(short = "C", long, name = "path")]
    #[structopt(verbatim_doc_comment)]
    directory: Option<PathBuf>,

    /// The program to convert FLT files to VHS
    /// Assumed usage is `<converter> input.flt`
    #[structopt(long, default_value = "flt2vhs.exe", name = "program")]
    #[structopt(verbatim_doc_comment)]
    converter: PathBuf,

    /// Don't convert FLT files to VHS once they've been moved.
    #[structopt(short, long)]
    no_convert: bool,

    /// Keep FLT files instead of deleting them after converting them.
    /// Ignored if --no-convert is given
    #[structopt(short, long, verbatim_doc_comment)]
    keep: bool,
}

fn main() {
    run().unwrap_or_else(|e| {
        error!("{:?}", e);
        std::process::exit(1);
    });
}

fn run() -> Result<()> {
    let args = Args::from_args();
    logsetup::init_logger(std::cmp::max(1, args.verbose), args.timestamps, args.color);

    if let Some(change_to) = &args.directory {
        env::set_current_dir(change_to).with_context(|| {
            format!("Couldn't set working directory to {}", change_to.display())
        })?;
    }

    if !args.keep {
        delete_garbage_files()?;
    }
    rename_and_convert(&args)
}

fn delete_garbage_files() -> Result<()> {
    let delete_fail_warning = |path: &Path, e| {
        warn!(
            "Couldn't remove {} (are you running BMS?): {}",
            path.display(),
            e
        )
    };

    let cwd = env::current_dir()?;
    for f in fs::read_dir(cwd)? {
        let entry = f?;
        let path = entry.path();
        let meta = entry
            .metadata()
            .with_context(|| format!("Couldn't stat {}", path.display()))?;

        if path.to_str().map(|s| s.ends_with(".flt.cnt")).unwrap_or(false) && meta.is_file() {
            fs::remove_file(&path).unwrap_or_else(|e| delete_fail_warning(&path, e));
            info!("Deleted {}", path.display());
            continue;
        }

        // With -acmi, BMS 4.35 likes to spit out a bunch of tiny useless files in 2D.
        if path.extension() == Some(OsStr::new("flt")) && meta.is_file() {
            let length = meta.len();
            if length <= 34 {
                fs::remove_file(&path).unwrap_or_else(|e| delete_fail_warning(&path, e));
                info!("Deleted tiny {}-byte garbage {}", length, path.display());
            }
        }
    }
    Ok(())
}

fn rename_and_convert(args: &Args) -> Result<()> {
    let mut to_rename = Vec::new();

    let cwd = env::current_dir()?;
    for f in fs::read_dir(cwd)? {
        let entry = f?;
        let path = entry.path();
        if path.extension() == Some(OsStr::new("flt")) && entry.metadata()?.is_file() {
            trace!("Found {} to rename and convert", path.display());
            to_rename.push(path);
        }
    }

    if to_rename.is_empty() {
        info!("Nothing to convert!");
        return Ok(());
    }

    let mut renamed_flights = to_rename
        .iter()
        .map(|f| rename_flt(f))
        .collect::<Result<Vec<_>>>()?;

    // Make sure we pass files in sorted order. This should ensure
    // <timestamp>-acmi0000.flt comes before <timestamp>-acmi0001.flt, etc.,
    // which should ensure we merge files in the correct order.
    renamed_flights.sort();
    convert_flts(args, &renamed_flights)?;
    Ok(())
}

fn rename_flt(to_rename: &Path) -> Result<PathBuf> {
    let rename_to = timestamp_name(to_rename);
    fs::rename(&to_rename, &rename_to)
        .with_context(|| format!("Renaming {} failed", to_rename.display()))?;
    info!("Renamed {} to {}", to_rename.display(), rename_to.display());
    Ok(rename_to)
}

const MOVED_SUFFIX: &str = ".moved";

fn timestamp_name(to_rename: &Path) -> PathBuf {
    use std::os::windows::fs::MetadataExt;

    let now = Local::now();

    // Try to avoid stomping other files by using to_rename as part of the output.
    // Avoids cases (like copying several at once) where several were created
    // in the same second and get moved on top of each other.
    match fs::metadata(to_rename).map(|meta| windows_timestamp(meta.creation_time())) {
        Ok(Some(ct)) => {
            let local = ct.with_timezone(now.offset());
            PathBuf::from(format!(
                "{}_{}.flt{}",
                local.format("%Y-%m-%d_%H-%M-%S"),
                to_rename.file_stem().unwrap().to_string_lossy(),
                MOVED_SUFFIX
            ))
        }
        Ok(None) | Err(_) => {
            trace!(
                "Couldn't get creation time of {}. Falling back to current time",
                to_rename.display()
            );
            PathBuf::from(format!(
                "{}_{}.flt{}",
                now.format("%Y-%m-%d_%H-%M-%S_{}"),
                to_rename.file_stem().unwrap().to_string_lossy(),
                MOVED_SUFFIX
            ))
        }
    }
}

fn windows_timestamp(ts: u64) -> Option<DateTime<Utc>> {
    // Windows returns 100ns intervals since January 1, 1601
    const TICKS_PER_SECOND: u64 = 1_000_000_000 / 100;

    if ts == 0 {
        None
    } else {
        let seconds = ts / TICKS_PER_SECOND;
        let nanos = (ts % TICKS_PER_SECOND) * 100;

        Some(
            Utc.ymd(1601, 1, 1).and_hms(0, 0, 0)
                + chrono::Duration::seconds(seconds as i64)
                + chrono::Duration::nanoseconds(nanos as i64),
        )
    }
}

fn path_list(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| p.to_string_lossy())
        .collect::<Vec<_>>()
        .join(", ")
}

fn convert_flts(args: &Args, flts: &[PathBuf]) -> Result<()> {
    debug!(
        "Converting {:?} with {}",
        path_list(flts),
        args.converter.display()
    );

    let mut proc = std::process::Command::new(&args.converter);
    // Add a verbosity flag if it's flt2vhs.
    // Don't for other programs since we shouldn't assume how their flags work
    if args.converter == Path::new("flt2vhs.exe") {
        proc.arg("-v");
        if !args.keep {
            proc.arg("--delete");
        }
    }
    proc.args(flts);
    let exit_status = proc
        .status()
        .with_context(|| format!("Couldn't run {}", args.converter.display()))?;
    if exit_status.success() {
        Ok(())
    } else {
        bail!(
            "{} failed to convert {}",
            args.converter.display(),
            path_list(flts)
        );
    }
}
