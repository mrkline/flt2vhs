use std::io::{Read, Result};

/// Reads a little-endian u32 from the front of the provided reader
#[inline]
pub fn read_u32<R: Read>(r: &mut R) -> Result<u32> {
    let mut bytes: [u8; 4] = [0; 4];
    r.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

/// Reads a little-endian i32 from the front of the provided reader
#[inline]
pub fn read_i32<R: Read>(r: &mut R) -> Result<i32> {
    let mut bytes: [u8; 4] = [0; 4];
    r.read_exact(&mut bytes)?;
    Ok(i32::from_le_bytes(bytes))
}

/// Reads a little-endian f32 from the front of the provided reader
#[inline]
pub fn read_f32<R: Read>(r: &mut R) -> Result<f32> {
    let mut bytes: [u8; 4] = [0; 4];
    r.read_exact(&mut bytes)?;
    Ok(f32::from_le_bytes(bytes))
}
