use std::fs;
use std::path::{Path, PathBuf};

use anyhow::*;
use log::*;
use structopt::StructOpt;

/// Patch BMS to not convert FLT to VHS files
#[derive(Debug, StructOpt)]
#[structopt(verbatim_doc_comment)]
struct Args {
    /// Verbosity (-v, -vv, -vvv, etc.)
    #[structopt(short, long, parse(from_occurrences))]
    verbose: u8,

    #[structopt(short, long, case_insensitive = true, default_value = "auto")]
    #[structopt(name = "always/auto/never")]
    color: logsetup::Color,

    /// The BMS executable to read
    #[structopt(name = "Falcon BMS.exe")]
    input: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::from_args();
    logsetup::init_logger(args.verbose, false, args.color)?;

    let bms_path = find_bms(args.input)?;
    let map = open_bms(&bms_path)?;
    ensure_bms(&map)?;

    ensure!(
        map.len() >= 0x004DCF6D,
        "EXE is too short - are you sure this is BMS 4.35U1?"
    );

    let call_to_nop = &map[0x004DCF68..0x004DCF6D];
    println!("{:x?}", call_to_nop);

    Ok(())
}

fn find_bms(input: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(i) = input {
        return Ok(i);
    }

    // Check the registry if we can.
    #[cfg(windows)]
    {
        match find_bms_from_registry() {
            Err(e) => warn!("Couldn't find BMS from registry: {:?}", e),
            ok => return ok,
        };
    }

    debug!("Last try: Assuming we're in BMS/User/Acmi. Let's look in Bin...");
    let last_try = Path::new("../../Bin/x64/Falcon BMS.exe");
    if last_try.exists() {
        Ok(last_try.to_owned())
    } else {
        bail!("Couldn't find Falcon BMS.exe");
    }
}

#[cfg(windows)]
fn find_bms_from_registry() -> Result<PathBuf> {
    use registry::*;

    let key = Hive::LocalMachine
        .open(
            r"SOFTWARE\WOW6432Node\Benchmark Sims\Falcon BMS 4.35",
            Security::Read,
        )
        .context("Couldn't find BMS registry key")?;

    match key
        .value("baseDir")
        .context("Couldn't find BMS baseDir registry value")?
    {
        Data::String(wide) => Ok(Path::new(&wide.to_os_string()).join("Bin/x64/Falcon BMS 4.35")),
        _ => bail!("Expected a string for BMS baseDir, got something else"),
    }
}

fn open_bms(b: &Path) -> Result<memmap::MmapMut> {
    let fh = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(b)
        .with_context(|| format!("Couldn't open {}", b.display()))?;
    let mapping = unsafe { memmap::MmapMut::map_mut(&fh) }
        .with_context(|| format!("Couldn't memory map {}", b.display()))?;

    Ok(mapping)
}

fn ensure_bms(map: &[u8]) -> Result<()> {
    use pelite::pe64::{Pe, PeFile};

    let bin = PeFile::from_bytes(map).context("Couldn't load file as an EXE")?;

    let resources = bin.resources()?;
    let version_info = resources.version_info()?;

    // Get the first available language
    let lang = version_info.translation()[0];

    // Is this BMS?
    let product_name = version_info
        .value(lang, "ProductName")
        .ok_or_else(|| anyhow!("Couldn't get EXE name"))?;
    ensure!(
        product_name == "Falcon BMS",
        "EXE says it's {}, not Falcon BMS",
        product_name
    );

    let version = version_info
        .value(lang, "ProductVersion")
        .ok_or_else(|| anyhow!("Couldn't get EXE version"))?;
    ensure!(
        version == "4.35.1",
        "Only BMS 4.35.1 is currently supported, got {}",
        version
    );

    ensure!(
        map.len() == 81105920,
        "EXE isn't the right size - are you sure this is BMS 4.35U1?"
    );
    Ok(())
}
