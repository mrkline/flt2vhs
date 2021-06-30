use std::fs;
use std::path::{Path, PathBuf};

use anyhow::*;
use log::*;
use structopt::StructOpt;

/// Patch BMS to not convert FLT to VHS files when leaving 3D
#[derive(Debug, StructOpt)]
#[structopt(verbatim_doc_comment)]
struct Args {
    /// Verbosity (-v, -vv, -vvv, etc.)
    #[structopt(short, long, parse(from_occurrences))]
    verbose: u8,

    #[structopt(short, long, case_insensitive = true, default_value = "auto")]
    #[structopt(name = "always/auto/never")]
    color: logsetup::Color,

    /// Restore BMS to its original state
    /// (convert VHS files when leaving 3D)
    #[structopt(short, long, verbatim_doc_comment)]
    restore: bool,

    /// The BMS executable to patch.
    /// If unspecified, will check the registry for the BMS path.
    #[structopt(name = "Falcon BMS.exe", verbatim_doc_comment)]
    input: Option<PathBuf>,
}

enum BmsExeVersion {
    Ver4_35_1,
    Ver4_35_2,
}

struct Patch {
    offset: usize,
    original: &'static [u8],
    replacement: &'static [u8],
}

fn main() {
    run().unwrap_or_else(|e| {
        error!("{:?}", e);
        std::process::exit(1);
    });
}

fn run() -> Result<()> {
    let args = Args::from_args();
    logsetup::init_logger(std::cmp::max(1, args.verbose), false, args.color);

    let bms_path = find_bms(args.input)?;
    let mut map = open_bms(&bms_path)?;
    let bms_version = find_bms_version(&map)?;

    const REPLACEMENT_NOP: &[u8] = &[0x0F, 0x1F, 0x44, 0x00, 0x00]; // lea eax, eax * 1 + 0

    let patches_435u1 = vec![
        Patch {
            offset: 0x0002_2544,
            original: &[0xE8, 0x87, 0x55, 0x00, 0x00],
            replacement: REPLACEMENT_NOP,
        },
        Patch {
            offset: 0x004D_CF68,
            original: &[0xE8, 0x63, 0xAB, 0xB4, 0xFF],
            replacement: REPLACEMENT_NOP,
        },
    ];

    let patches_435u2 = vec![
        Patch {
            offset: 0x0001_CA12,
            original: &[0xE8, 0x99, 0x55, 0x00, 0x00],
            replacement: REPLACEMENT_NOP,
        },
        Patch {
            offset: 0x004F_B5C8,
            original: &[0xE8, 0xE3, 0x69, 0xB2, 0xFF],
            replacement: REPLACEMENT_NOP,
        },
    ];

    let patches = match bms_version {
        BmsExeVersion::Ver4_35_1 => patches_435u1,
        BmsExeVersion::Ver4_35_2 => patches_435u2,
    };

    for patch in &patches {
        patch_call(&mut map, patch, args.restore)?;
    }

    map.flush()
        .context("Couldn't save changes to Falcon BMS.exe")?;

    if args.restore {
        info!("BMS restored to its original state")
    } else {
        info!("FLT -> VHS conversion removed");
    }

    Ok(())
}

fn patch_call(map: &mut [u8], patch: &Patch, restore: bool) -> Result<()> {
    assert_eq!(patch.original.len(), patch.replacement.len());

    let patch_len = patch.original.len();
    let call_to_nop = &mut map[patch.offset..patch.offset + patch_len];
    ensure!(
        call_to_nop.len() == patch_len,
        "EXE is too short - are you sure this is BMS 4.35U1?"
    );

    #[allow(clippy::collapsible_else_if)]
    if restore {
        if call_to_nop == patch.original {
            debug!(
                "ACMI_ImportFile call at {:08X} is unmodified; nothing to do!",
                patch.offset
            );
        } else if call_to_nop == patch.replacement {
            debug!("Restoring call to ACMI_ImportFile at {:08X}", patch.offset);
            call_to_nop.copy_from_slice(patch.original);
        } else {
            bail!(
                "Unexpected bytes at {:X}: {:08X?}",
                patch.offset,
                call_to_nop
            );
        }
    } else {
        if call_to_nop == patch.original {
            debug!(
                "Replacing call to ACMI_ImportFile at {:08X} with no-op",
                patch.offset
            );
            call_to_nop.copy_from_slice(patch.replacement);
        } else if call_to_nop == patch.replacement {
            debug!(
                "ACMI_ImportFile call at {:08X} is already no-op'd; nothing to do!",
                patch.offset
            );
        } else {
            bail!(
                "Unexpected bytes at {:08X}: {:x?}",
                patch.offset,
                call_to_nop
            );
        }
    }
    Ok(())
}

fn find_bms(input: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(i) = input {
        debug!("User gave {} as the Falcon BMS.exe path", i.display());
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

    debug!("Last try: Assuming we're in BMS/User/Acmi. Let's look in BMS/Bin...");
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

    debug!("Looking for Falcon BMS.exe in the registry");

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
        Data::String(wide) => Ok(Path::new(&wide.to_os_string()).join("Bin/x64/Falcon BMS.exe")),
        _ => bail!("Expected a string for BMS baseDir, got something else"),
    }
}

fn open_bms(bms: &Path) -> Result<memmap::MmapMut> {
    info!("Opening {}", bms.display());
    let fh = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(bms)
        .with_context(|| format!("Couldn't open {}", bms.display()))?;
    let mapping = unsafe { memmap::MmapMut::map_mut(&fh) }
        .with_context(|| format!("Couldn't memory map {}", bms.display()))?;

    Ok(mapping)
}

fn find_bms_version(map: &[u8]) -> Result<BmsExeVersion> {
    use pelite::pe64::{Pe, PeFile};

    info!("Determining BMS version.");

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

    const SUPPORTED_VERSIONS: &[&str] = &["4.35.1", "4.35.2"];
    let version_field = version_info
        .value(lang, "ProductVersion")
        .ok_or_else(|| anyhow!("Couldn't get EXE version"))?;
    let vs = version_field.as_str();

    info!("Version detecteed: {}", version_field);
    ensure!(
        SUPPORTED_VERSIONS.contains(&vs),
        "Detected BMS version {} not supported. Supported versions are {:?}.",
        vs,
        SUPPORTED_VERSIONS
    );

    let version = match vs {
        "4.35.1" => BmsExeVersion::Ver4_35_1,
        "4.35.2" => BmsExeVersion::Ver4_35_2,
        _ => unreachable!(format!("version that was detected {} is invalid.", vs)),
    };

    let expected_exe_size = match version {
        BmsExeVersion::Ver4_35_1 => 81105920,
        BmsExeVersion::Ver4_35_2 => 164310528,
    };

    ensure!(
        map.len() == expected_exe_size,
        "EXE isn't the right size - are you sure this is BMS version {}?",
        vs
    );
    Ok(version)
}
