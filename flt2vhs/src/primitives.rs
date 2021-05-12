///! Utility functions for reading and writing primitives
use std::io::{Result, Read, Write};

/// Writes a byte to the provided writer
#[inline(always)]
pub fn write_u8<W: Write>(b: u8, w: &mut W) -> Result<()> {
    w.write_all(&[b])
}

/// Reads a little-endian u32 from the provided reader
#[inline]
pub fn read_u32<R: Read>(r: &mut R) -> Result<u32> {
    let mut bytes: [u8; 4] = [0; 4];
    r.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

/// Writes a little-endian u32 to the provided writer
#[inline]
pub fn write_u32<W: Write>(i: u32, w: &mut W) -> Result<()> {
    let bytes = i.to_le_bytes();
    w.write_all(&bytes)
}

/// Reads a little-endian i32 from the provided reader
#[inline]
pub fn read_i32<R: Read>(r: &mut R) -> Result<i32> {
    let mut bytes: [u8; 4] = [0; 4];
    r.read_exact(&mut bytes)?;
    Ok(i32::from_le_bytes(bytes))
}

/// Writes a little-endian i32 to the provided writer
#[inline]
pub fn write_i32<W: Write>(i: i32, w: &mut W) -> Result<()> {
    let bytes = i.to_le_bytes();
    w.write_all(&bytes)
}

/// Reads a little-endian f32 from the provided reader
#[inline]
pub fn read_f32<R: Read>(r: &mut R) -> Result<f32> {
    let mut bytes: [u8; 4] = [0; 4];
    r.read_exact(&mut bytes)?;
    Ok(f32::from_le_bytes(bytes))
}

/// Writes a little-endian f32 to the provided writer
#[inline]
pub fn write_f32<W: Write>(f: f32, w: &mut W) -> Result<()> {
    let bytes = f.to_le_bytes();
    w.write_all(&bytes)
}
