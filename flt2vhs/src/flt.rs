use std::io;
use std::io::prelude::*;

use anyhow::*;
use log::*;

use crate::read_primitives::*;

/// Information parsed from a .flt file, needed to make a .vhs file
#[derive(Debug, Clone, Default)]
pub struct Flight {
    pub corrupted: bool,
    pub callsigns: Vec<CallsignRecord>,
}

const REC_TYPE_GENERAL_POSITION: u8 = 0;
const REC_TYPE_MISSILE_POSITION: u8 = 1;
const REC_TYPE_FEATURE_POSITION: u8 = 2;
const REC_TYPE_AIRCRAFT_POSITION: u8 = 3;
const REC_TYPE_TRACER_START: u8 = 4;
const REC_TYPE_STATIONARY_SFX: u8 = 5;
const REC_TYPE_MOVING_SFX: u8 = 6;
const REC_TYPE_SWITCH: u8 = 7;
const REC_TYPE_DOF: u8 = 8;
const REC_TYPE_CHAFF_POSITION: u8 = 9;
const REC_TYPE_FLARE_POSITION: u8 = 10;
const REC_TYPE_TOD_OFFSET: u8 = 11;
const REC_TYPE_FEATURE_STATUS: u8 = 12;
const REC_TYPE_CALLSIGN_LIST: u8 = 13;

impl Flight {
    pub fn parse<R: Read>(mut r: R) -> Self {
        let mut flight = Self { .. Default::default() };

        loop {
            match read_record(&mut flight, &mut r) {
                Ok(true) => { /* continue */ },
                Ok(false) => break,
                Err(e) => {
                    warn_on(e);
                    flight.corrupted = true;
                    break;
                }
            }
        }
        flight
    }
}

fn read_record<R: Read>(flight: &mut Flight, r: &mut R) -> Result<bool> {
    let mut type_byte: [u8; 1] = [0];
    match r.read(&mut type_byte) {
        Ok(0) => return Ok(false), // EOF
        Ok(1) => {},
        Ok(_) => unreachable!(),
        Err(e) => return Err(Error::new(e)),
    };

    match type_byte[0] {
        REC_TYPE_CALLSIGN_LIST => flight.callsigns = parse_callsigns(r)?,
        wut => {
            bail!("Unknown enity type {} (0-13 are valid)", wut);
        }
    };
    Ok(true)
}

fn warn_on(e: Error) {
    if let Some(io_err) = e.downcast_ref::<io::Error>() {
        if io_err.kind() == io::ErrorKind::UnexpectedEof {
            warn!("Reached end of file in the middle of a record");
            return;
        }
    }
    warn!("Error reading flight: {}", e);
}

fn parse_callsigns<R: Read>(r: &mut R) -> Result<Vec<CallsignRecord>> {
    let callsign_count = read_i32(r)?;
    ensure!(callsign_count >= 0, "Negative ({}) callsign count!", callsign_count);

    let mut callsigns = Vec::new();

    for _ in 0..callsign_count {
        callsigns.push(CallsignRecord::read(r)?);
    }
    Ok(callsigns)
}

#[derive(Debug, Clone)]
pub struct CallsignRecord {
    pub label: [u8; 16],
    pub team_color: i32,
}

impl CallsignRecord {
    pub fn read<R: Read>(r: &mut R) -> Result<Self> {
        let mut label: [u8; 16] = [0; 16];
        r.read_exact(&mut label)?;
        let team_color = read_i32(r)?;
        Ok(Self { label, team_color })
    }
}
