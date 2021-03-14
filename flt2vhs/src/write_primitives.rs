use std::io::{Result, Write};

/// Writes a byte to the provided writer
#[inline(always)]
pub fn write_u8<W: Write>(b: u8, w: &mut W) -> Result<()> {
    w.write_all(&[b])
}

/// Writes a little-endian u32 to the provided writer
#[inline]
pub fn write_u32<W: Write>(i: u32, w: &mut W) -> Result<()> {
    let bytes = i.to_le_bytes();
    w.write_all(&bytes)
}

/// Writes a little-endian i32 to the provided writer
#[inline]
pub fn write_i32<W: Write>(i: i32, w: &mut W) -> Result<()> {
    let bytes = i.to_le_bytes();
    w.write_all(&bytes)
}

/// Writes a little-endian f32 to the provided writer
#[inline]
pub fn write_f32<W: Write>(f: f32, w: &mut W) -> Result<()> {
    let bytes = f.to_le_bytes();
    w.write_all(&bytes)
}
