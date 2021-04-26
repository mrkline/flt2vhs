# flt2vhs

A set of tools to convert
[Falcon BMS](https://www.benchmarksims.org/forum/content.php) recordings
from their initial format (`.flt`) to their replay format (`.vhs`)
in seconds, not minutes.

## How do I use it?

1. Extract the 7zip archive into `BMS/User/Acmi`.

2. Run `patch-bms-novhs.exe` to disable BMS's (slow) FLT to VHS conversion.
   You only need to do this once.

3. Record BMS flights normally with the AVTR switch in the cockpit,
   or by running BMS with `-acmi`. BMS should no longer pause to convert
   recordings when you exit 3D.

4. Run `convert-all-flts.exe` to convert all FLT files to VHS,
   with names based on the times they were created.
   Or drag a FLT file file onto `flt2vhs.exe` to convert one at a time.

For the CLI-inclined, see each tool's `--help` for more options.

## Why?

Running BMS with the `-acmi` flag or flipping the AVTR switch in the cockpit
records your flight, but before it can be viewed in tools like
[Tacview](https://www.tacview.net/product/en/), it needs to be converted
from one format (FLT) to another (VHS).

As of version 4.35U1, BMS's conversion is painfully slow.
For 30+ minute flights with lots of planes and vehicles moving around,
the conversion takes several minutes, during which you stare at a black screen.

## What?

Since 2017, Loitho has provided a third-party tool called
[ACMI-Compiler](https://github.com/loitho/acmi-compiler) that:

1. Steals the FLT file from BMS in the brief time between when the game
   finishes writing it and when the game re-opens the file to start the conversion.

2. Performs the FLT -> VHS conversion itself in seconds.

These project does much of the same, but with major improvments:

1. **Nothing to run in the background:** Instead of running a background program
   to steal FLT files from BMS, flt2vhs ships with a tool (`patch-bms-novhs`)
   that just disables BMS's slow conversion.

2. **Even better performance:**

    - flt2vhs [memory maps](https://en.wikipedia.org/wiki/Memory-mapped_file#Benefits)
      the FLT file. This improves performance by reading directly out of the
      operating system's page cache instead of copying file data with each
      `read()` [system call](https://en.wikipedia.org/wiki/System_call).

    - ACMI-Compiler stores events in a series of large arrays, very similar to
      how they are stored in the VHS file. This simplifies actually writing the VHS,
      but complicates everything else. Nearly _all_ of the data has to be sorted,
      and many steps need to search through the arrays to find the data they need.

      flt2vhs organizes its data more efficiently.
      Events for each entity (plane, vehicle, missile, etc.) are stored in a
      [hash table](https://en.wikipedia.org/wiki/Hash_table), allowing us to
      look entities up in constant time instead of performing a search.
      This also reduces the amount of duplicated data - for example,
      events no longer needs to store the ID of the entity they belong to.
      Less data means the program is more cache-friendly, which is one of the
      [most important ways you can improve performance on modern systems.](https://www.youtube.com/watch?v=0_Byw9UMn9g)

    These design choices make flt2vhs very fast - about 30% faster than
    ACMI-Compiler according to initial benchmarks.
    Once the FLT file is parsed, only a few tenths of a second are spent
    computing the VHS output, and the rest of the time is spent waiting for the
    OS to put the file on the disk.

Additionally,

1. In the spirit of the [Unix philosophy](https://en.wikipedia.org/wiki/Unix_philosophy),
   "make each program do one thing well", functionality is split into a couple programs:
   `patch-bms-novhs` patches BMS, `flt2vhs` handles the actual FLT to VHS conversion,
   and `convert-all-flts` runs `flt2vhs` on each FLT file in the directory
   (including any in-progress ones from BMS).
   A tool to print VHS files as JSON, `vhscat`, is also provided for debugging.

2. Everything but `convert-all-flts` is entirely cross-platform and can be
   built/run/tested on Linux or MacOS.

## Thanks

This wouldn't be possible without Loitho.
The ACMI-Compiler source - and his kind responses to my many silly questions -
were instrumental to understanding the FLT and VHS formats.
The legwork they [went through](https://www.benchmarksims.org/forum/showthread.php?32245-Beta-ACMI-compiler&highlight=acmi+compiler)
to understand the formats in the first place is nothing short of impressive.
Loitho was also kind enough to provide example FLT files and expected outputs,
which gave me great test data.
