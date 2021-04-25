use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::io::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::mpsc::*;

use anyhow::*;
use chrono::prelude::*;
use log::*;
use notify::{raw_watcher, RecursiveMode, Watcher};
use structopt::StructOpt;

/// Monitors a directory and moves FLT files out from under BMS,
/// then calls another program to convert them to VHS
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

fn main() -> Result<()> {
    let args = Args::from_args();
    logsetup::init_logger(std::cmp::max(1, args.verbose), args.timestamps, args.color)?;

    if let Some(change_to) = &args.directory {
        env::set_current_dir(change_to).with_context(|| {
            format!("Couldn't set working directory to {}", change_to.display())
        })?;
    }

    rename_flts(&args)?;

    let (tx, rx) = channel();
    let mut watcher = raw_watcher(tx).unwrap();
    watcher
        .watch(env::current_dir()?, RecursiveMode::NonRecursive)
        .context("Couldn't start watching the current directory")?;

    loop {
        match rx.recv_timeout(std::time::Duration::from_secs(1)) {
            // We don't actually care what the event was;
            // rescanning is cheap.
            Ok(_) | Err(RecvTimeoutError::Timeout) => rename_flts(&args)?,
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

fn find_first_flt() -> Result<Option<PathBuf>> {
    for f in fs::read_dir(env::current_dir()?)? {
        let entry = f?;
        let path = entry.path();
        if path.extension() == Some(OsStr::new("flt")) && entry.metadata()?.is_file() {
            trace!("Found .flt file {}", path.display());
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn rename_flts(args: &Args) -> Result<()> {
    while let Some(flt) = find_first_flt()? {
        let renamed_to = rename_flt(flt)?;
        if !args.no_convert {
            if let Err(e) = convert_flt(args, &renamed_to) {
                warn!("{:?}", e);
            }
        }
    }
    Ok(())
}

fn rename_flt(to_rename: PathBuf) -> Result<PathBuf> {
    use std::os::windows::fs::OpenOptionsExt;

    // At first, let's just try to add the suffix ".moved"
    // to files. This gets them out of the way of subsequent searches,
    // and if they're not being actively recorded by BMS,
    // a timestamp name doesn't make a lot of sense.
    let mut rename_to = new_name(&to_rename, Naming::SuffixOnly);

    debug!("Trying to rename {}...", to_rename.display());
    match fs::rename(&to_rename, &rename_to) {
        Ok(()) => {
            info!("Renamed {} to {}", to_rename.display(), rename_to.display());
            return Ok(rename_to);
        }
        // Windows error 32: The file is being used by another process.
        Err(e) if e.raw_os_error() == Some(32) => {
            info!(
                "{} is in use (presumably by BMS). Waiting...",
                to_rename.display()
            );
        }
        Err(other_error) => {
            return Err(Error::from(other_error).context("Renaming failed unexpectedly"))
        }
    }

    // Try to open the .flt file in a loop - there's a brief window when BMS
    // finishes writing it before it opens it back up to convert to VHS.
    let mut flt_fh = loop {
        // This seems a bit long, but it's what acmi-compiler does.
        std::thread::sleep(std::time::Duration::from_millis(250));

        let open_result = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&to_rename);

        match open_result {
            Ok(fh) => {
                trace!("Opened {}", to_rename.display());
                break fh;
            }
            // Windows error 32: The file is being used by another process.
            Err(e) if e.raw_os_error() == Some(32) => {
                trace!(
                    "Couldn't open {}, still in use. Trying again shortly...",
                    to_rename.display()
                );
            }
            Err(other_error) => {
                return Err(Error::from(other_error)
                    .context(format!("Couldn't open {}", to_rename.display())))
            }
        }
    };

    // If it's a file that BMS was writing to, use the current timestamp.
    // This is more helpful than "acmi0000.flt" and also avoids the problem
    // of subsequent renames overwriting "acmi0000.flt.moved"
    rename_to = new_name(&to_rename, Naming::ByDate);

    // Closing the .flt file handle and doing a rename is a bit racy -
    // it assumes that BMS has given up on trying to open it in the meantime.
    // Instead, just do a copy.
    let mut renamed_fh = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&rename_to)
        .with_context(|| format!("Couldn't open {} for copying", rename_to.display()))?;

    io::copy(&mut flt_fh, &mut renamed_fh)?;
    renamed_fh.flush()?;

    // Copy successful!
    info!("Renamed {} to {}", to_rename.display(), rename_to.display());
    drop(flt_fh);
    fs::remove_file(to_rename)?;

    Ok(rename_to)
}

#[derive(Debug, Copy, Clone)]
enum Naming {
    /// Just add `.moved`
    SuffixOnly,
    /// Rename to `<timestamp>.flt.moved`
    ByDate,
}

const MOVED_SUFFIX: &str = ".moved";

fn new_name(to_rename: &Path, naming: Naming) -> PathBuf {
    match naming {
        Naming::SuffixOnly => {
            let mut with_suffix = to_rename.to_owned().into_os_string();
            with_suffix.push(MOVED_SUFFIX);
            PathBuf::from(with_suffix)
        }
        Naming::ByDate => timestamp_name(to_rename),
    }
}

// _TOCTOU: The Function_, but let's assume nothing's making a bunch of FLT files
// in the exact same second.
fn timestamp_name(to_rename: &Path) -> PathBuf {
    use std::os::windows::fs::MetadataExt;

    let now = Local::now();

    match fs::metadata(to_rename).map(|meta| windows_timestamp(meta.creation_time())) {
        Ok(Some(ct)) => {
            let local = ct.with_timezone(now.offset());
            PathBuf::from(format!(
                "{}.flt{}",
                local.format("%Y-%m-%d_%H-%M-%S"),
                MOVED_SUFFIX
            ))
        }
        Ok(None) | Err(_) => {
            trace!(
                "Couldn't get creation time of {}. Falling back to current time",
                to_rename.display()
            );
            PathBuf::from(format!(
                "{}.flt{}",
                now.format("%Y-%m-%d_%H-%M-%S"),
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

fn convert_flt(args: &Args, flt: &Path) -> Result<()> {
    debug!(
        "Converting {} with {}",
        flt.display(),
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
    proc.arg(flt);
    let exit_status = proc
        .status()
        .with_context(|| format!("Couldn't run {}", args.converter.display()))?;
    if exit_status.success() {
        Ok(())
    } else {
        bail!(
            "{} failed to convert {}",
            args.converter.display(),
            flt.display()
        );
    }
}
