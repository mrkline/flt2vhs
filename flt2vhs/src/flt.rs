use std::cmp::Ordering;
use std::collections::BTreeMap;
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
    pub tod_offset: f32,
    pub entities: BTreeMap<Entity, EntityUpdates>,
    pub features: BTreeMap<Entity, FeatureUpdates>,
    pub general_events: Vec<GeneralEvent>,
}

impl Flight {
    pub fn parse<R: Read>(mut r: R) -> Self {
        let mut flight = Self {
            ..Default::default()
        };

        loop {
            match read_record(&mut flight, &mut r) {
                Ok(true) => { /* continue */ }
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

const ENTITY_FLAG_MISSILE: u32 = 0x00000001;
const ENTITY_FLAG_FEATURE: u32 = 0x00000002;
const ENTITY_FLAG_AIRCRAFT: u32 = 0x00000004;
const ENTITY_FLAG_CHAFF: u32 = 0x00000008;
const ENTITY_FLAG_FLARE: u32 = 0x00000010;

#[derive(Debug, Copy, Clone, Default)]
pub struct Entity {
    pub uid: i32,
    pub kind: i32,
    pub flags: u32,
}

impl PartialEq for Entity {
    fn eq(&self, other: &Self) -> bool {
        self.uid == other.uid
    }
}
impl Eq for Entity {}

impl Ord for Entity {
    fn cmp(&self, other: &Self) -> Ordering {
        self.uid.cmp(&other.uid)
    }
}

impl PartialOrd for Entity {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Copy, Clone)]
pub struct PositionUpdate {
    pub time: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub pitch: f32,
    pub roll: f32,
    pub yaw: f32,
    /// UID of the current radar target
    pub radar_target: i32,
}

#[derive(Debug, Copy, Clone)]
pub struct EntityEvent {}

#[derive(Debug, Clone, Default)]
pub struct EntityUpdates {
    position_updates: Vec<PositionUpdate>,
    events: Vec<EntityEvent>,
}

#[derive(Debug, Copy, Clone)]
pub struct FeatureEvent {
    pub time: f32,
    pub new_status: i32,
    pub previous_status: i32,
}

#[derive(Debug, Clone, Default)]
pub struct FeatureUpdates {
    position_updates: Vec<PositionUpdate>,
    events: Vec<FeatureEvent>,
}

#[derive(Debug, Copy, Clone, Default)]
pub struct GeneralEvent {
    pub type_byte: u8,
    pub start: f32,
    pub stop: f32,
    pub kind: i32,
    pub scale: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub dx: f32,
    pub dy: f32,
    pub dz: f32,
    pub roll: f32,
    pub pitch: f32,
    pub yaw: f32,
}

pub const REC_TYPE_GENERAL_POSITION: u8 = 0;
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

fn read_record<R: Read>(flight: &mut Flight, r: &mut R) -> Result<bool> {
    let mut type_byte: [u8; 1] = [0];
    match r.read(&mut type_byte) {
        Ok(0) => return Ok(false), // EOF
        Ok(1) => {}
        Ok(_) => unreachable!(),
        Err(e) => return Err(Error::new(e)),
    };
    let type_byte = type_byte[0];

    let time = read_f32(r)?;

    match type_byte {
        REC_TYPE_GENERAL_POSITION
        | REC_TYPE_MISSILE_POSITION
        | REC_TYPE_AIRCRAFT_POSITION
        | REC_TYPE_CHAFF_POSITION
        | REC_TYPE_FLARE_POSITION => {
            let record = PositionRecord::parse(r)?;
            let radar_target = if type_byte == REC_TYPE_AIRCRAFT_POSITION {
                read_i32(r)?
            } else {
                -1
            };

            // Find the existing entity or create a new one, then append
            // this position update.
            let flags = match type_byte {
                REC_TYPE_GENERAL_POSITION => 0,
                REC_TYPE_MISSILE_POSITION => ENTITY_FLAG_MISSILE,
                REC_TYPE_AIRCRAFT_POSITION => ENTITY_FLAG_AIRCRAFT,
                REC_TYPE_CHAFF_POSITION => ENTITY_FLAG_CHAFF,
                REC_TYPE_FLARE_POSITION => ENTITY_FLAG_FLARE,
                _ => unreachable!(),
            };
            let entity = Entity {
                uid: record.uid,
                kind: record.kind,
                flags,
            };
            let updates = flight
                .entities
                .entry(entity)
                .or_insert_with(Default::default);

            let posit_update = PositionUpdate {
                time,
                x: record.x,
                y: record.y,
                z: record.z,
                pitch: record.pitch,
                roll: record.roll,
                yaw: record.yaw,
                radar_target,
            };
            trace!("Entity {:?} position update: {:?}", entity, posit_update);
            updates.position_updates.push(posit_update);
        }
        REC_TYPE_FEATURE_POSITION => {
            let record = PositionRecord::parse(r)?;
            let radar_target = -1; // Should this be 0 to match acmi-compiler?
            let flags = ENTITY_FLAG_FEATURE;

            let feature = Entity {
                uid: record.uid,
                kind: record.kind,
                flags,
            };
            let updates = flight
                .features
                .entry(feature)
                .or_insert_with(Default::default);

            let posit_update = PositionUpdate {
                time,
                x: record.x,
                y: record.y,
                z: record.z,
                pitch: record.pitch,
                roll: record.roll,
                yaw: record.yaw,
                radar_target,
            };
            trace!("Feature {:?} position update: {:?}", feature, posit_update);
            updates.position_updates.push(posit_update);
        }
        REC_TYPE_TRACER_START => {
            let record = TracerStartRecord::parse(r)?;
            let event = GeneralEvent {
                type_byte,
                start: time,
                stop: time + 2.5, // Matches acmi-compiler
                x: record.x,
                y: record.y,
                z: record.z,
                dx: record.dx,
                dy: record.dy,
                dz: record.dz,
                ..Default::default()
            };
            trace!("Tracer start: {:?}", event);
        }
        REC_TYPE_STATIONARY_SFX => {
            let record = StationarySoundRecord::parse(r)?;
            let event = GeneralEvent {
                type_byte,
                start: time,
                stop: time + record.ttl,
                kind: record.kind,
                x: record.x,
                y: record.y,
                z: record.z,
                scale: record.scale,
                ..Default::default()
            };
            trace!("Stationary sound: {:?}", event);
        }
        REC_TYPE_MOVING_SFX => {}
        REC_TYPE_SWITCH => {}
        REC_TYPE_DOF => {}
        REC_TYPE_TOD_OFFSET => flight.tod_offset = time,
        REC_TYPE_FEATURE_STATUS => {
            let record = FeatureEventRecord::read(r)?;
            // Look up the feature by its UID
            let lookup = Entity {
                uid: record.uid,
                kind: -1,
                flags: 0,
            };
            let feature = flight
                .features
                .get_mut(&lookup)
                .ok_or_else(|| anyhow!("Couldn't find feature {} to add an event", record.uid))?;
            let event = FeatureEvent {
                time,
                new_status: record.new_status,
                previous_status: record.previous_status,
            };
            trace!("Feature {:?} event: {:?}", record.uid, event);
            feature.events.push(event);
        }
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

#[derive(Debug)]
struct PositionRecord {
    kind: i32,
    uid: i32,
    x: f32,
    y: f32,
    z: f32,
    yaw: f32,
    pitch: f32,
    roll: f32,
}

impl PositionRecord {
    fn parse<R: Read>(r: &mut R) -> Result<Self> {
        let kind = read_i32(r)?;
        let uid = read_i32(r)?;
        let x = read_f32(r)?;
        let y = read_f32(r)?;
        let z = read_f32(r)?;
        let yaw = read_f32(r)?;
        let pitch = read_f32(r)?;
        let roll = read_f32(r)?;

        Ok(Self {
            kind,
            uid,
            x,
            y,
            z,
            yaw,
            pitch,
            roll,
        })
    }
}

#[derive(Debug)]
struct TracerStartRecord {
    x: f32,
    y: f32,
    z: f32,
    dx: f32,
    dy: f32,
    dz: f32,
}

impl TracerStartRecord {
    fn parse<R: Read>(r: &mut R) -> Result<Self> {
        let x = read_f32(r)?;
        let y = read_f32(r)?;
        let z = read_f32(r)?;
        let dx = read_f32(r)?;
        let dy = read_f32(r)?;
        let dz = read_f32(r)?;

        Ok(Self {
            x,
            y,
            z,
            dx,
            dy,
            dz,
        })
    }
}

#[derive(Debug)]
struct StationarySoundRecord {
    kind: i32,
    x: f32,
    y: f32,
    z: f32,
    ttl: f32,
    scale: f32,
}

impl StationarySoundRecord {
    fn parse<R: Read>(r: &mut R) -> Result<Self> {
        let kind = read_i32(r)?;
        let x = read_f32(r)?;
        let y = read_f32(r)?;
        let z = read_f32(r)?;
        let ttl = read_f32(r)?;
        let scale = read_f32(r)?;

        Ok(Self {
            kind,
            x,
            y,
            z,
            ttl,
            scale,
        })
    }
}

#[derive(Debug)]
struct FeatureEventRecord {
    uid: i32,
    new_status: i32,
    previous_status: i32,
}

impl FeatureEventRecord {
    fn read<R: Read>(r: &mut R) -> Result<Self> {
        let uid = read_i32(r)?;
        let new_status = read_i32(r)?;
        let previous_status = read_i32(r)?;
        Ok(Self {
            uid,
            new_status,
            previous_status,
        })
    }
}

fn parse_callsigns<R: Read>(r: &mut R) -> Result<Vec<CallsignRecord>> {
    let callsign_count = read_i32(r)?;
    ensure!(
        callsign_count >= 0,
        "Negative ({}) callsign count!",
        callsign_count
    );

    let mut callsigns = Vec::new();

    for _ in 0..callsign_count {
        callsigns.push(CallsignRecord::read(r)?);
    }
    Ok(callsigns)
}

#[derive(Debug, Copy, Clone)]
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
