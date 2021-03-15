use std::collections::HashMap;
use std::io;
use std::io::prelude::*;

use anyhow::*;
use log::*;

use crate::read_primitives::*;

/// Information parsed from a .flt file, needed to make a .vhs file
#[derive(Debug, Clone, Default)]
pub struct Flight {
    pub corrupted: bool,
    pub tod_offset: f32,
    pub start_time: f32,
    pub end_time: f32,
    pub callsigns: Vec<CallsignRecord>,
    pub entities: HashMap<i32, EntityData>,
    pub features: HashMap<i32, FeatureData>,
    pub general_events: Vec<GeneralEvent>,

    // Feature events aren't stored in FeatureData
    // (like entity events are in EntityData).
    //
    // They need to be written chronologically
    // (in the order they were in the `.flt` file),
    // so we'd have to flatten it all back out into a vector anyways.
    pub feature_events: Vec<FeatureEvent>,
}

impl Flight {
    pub fn parse<R: Read>(mut r: R) -> Self {
        let mut flight = Self {
            start_time: f32::NEG_INFINITY,
            end_time: f32::NEG_INFINITY,
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

        // Some entities get event, but never any position updates.
        // Very strange, but let's throw them out since we can't do anything
        // with events for a entity that was never defined.
        let entities_to_chuck = flight
            .entities
            .iter()
            .filter(|(_uid, data)| data.position_data.is_none())
            .map(|(uid, data)| (*uid, data.events.len() as u32))
            .collect::<Vec<(i32, u32)>>();
        if !entities_to_chuck.is_empty() {
            debug!(
                "{} entities were never defined with position info, but have {} events",
                entities_to_chuck.len(),
                entities_to_chuck
                    .iter()
                    .fold(0, |acc, (_uid, events)| acc + events)
            );
        }
        for (uid, _) in entities_to_chuck {
            assert!(flight.entities.remove(&uid).is_some());
        }

        flight
    }
}

const ENTITY_FLAG_MISSILE: u32 = 0x00000001;
pub const ENTITY_FLAG_FEATURE: u32 = 0x00000002;
const ENTITY_FLAG_AIRCRAFT: u32 = 0x00000004;
const ENTITY_FLAG_CHAFF: u32 = 0x00000008;
const ENTITY_FLAG_FLARE: u32 = 0x00000010;

#[derive(Debug, Copy, Clone)]
pub struct EntityPositionUpdate {
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
pub struct EntityEvent {
    pub time: f32,
    pub payload: EntityEventPayload,
}

#[derive(Debug, Copy, Clone)]
pub enum EntityEventPayload {
    SwitchEvent(SwitchEvent),
    DofEvent(DofEvent),
}

#[derive(Debug, Copy, Clone)]
pub struct SwitchEvent {
    pub switch_number: i32,
    pub new_switch_value: i32,
    pub previous_switch_value: i32,
}

#[derive(Debug, Copy, Clone)]
pub struct DofEvent {
    pub dof_number: i32,
    pub new_dof_value: f32,
    pub previous_dof_value: f32,
}

/// Data gleaned from the entity's first position record.
/// (Contains things besides the position)
#[derive(Debug, Clone)]
pub struct EntityPositionData {
    pub kind: i32,
    /// Stores The type of entity (see `ENTITY_FLAG_...`)
    pub flags: u32,
    pub position_updates: Vec<EntityPositionUpdate>,
}

#[derive(Debug, Clone, Default)]
pub struct EntityData {
    /// Sometimes events start arriving before the position data,
    /// so fill that in when it arrives.
    ///
    /// TODO: Some entities _never_ get position data; only events.
    ///       Should we ignore events that arrive before a position for that UID?
    pub position_data: Option<EntityPositionData>,
    pub events: Vec<EntityEvent>,
}

#[derive(Debug, Copy, Clone)]
pub struct FeatureEvent {
    pub time: f32,
    pub feature_uid: i32,
    pub new_status: i32,
    pub previous_status: i32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FeatureData {
    pub kind: i32,
    pub lead_uid: i32,
    pub slot: i32,
    pub special_flags: u32,
    pub time: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub pitch: f32,
    pub roll: f32,
    pub yaw: f32,
}

#[derive(Debug, Copy, Clone, Default)]
pub struct GeneralEvent {
    pub type_byte: u8,
    pub start: f32,
    pub stop: f32,
    pub kind: i32,
    pub user: i32,
    pub flags: u32,
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

    if type_byte != REC_TYPE_TOD_OFFSET {
        if flight.start_time < 0.0 {
            flight.start_time = time;
        }
        if flight.end_time < time {
            flight.end_time = time;
        }
    }

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
            let entity_data = flight
                .entities
                .entry(record.uid)
                .or_insert_with(Default::default);

            if let Some(posit_data) = &entity_data.position_data {
                if posit_data.kind != record.kind {
                    warn!(
                        "Position update for entity {} switched kinds from {} to {}",
                        record.uid, posit_data.kind, record.kind
                    );
                }

                if posit_data.flags != flags {
                    warn!(
                        "Position update for entity {} switched flags from {} to {}",
                        record.uid, posit_data.flags, flags
                    );
                }
            } else {
                trace!(
                    "New entity {}: kind {}, flags {}",
                    record.uid,
                    record.kind,
                    flags
                );
                entity_data.position_data = Some(EntityPositionData {
                    kind: record.kind,
                    flags,
                    position_updates: Vec::new(),
                });
            }

            let posit_update = EntityPositionUpdate {
                time,
                x: record.x,
                y: record.y,
                z: record.z,
                pitch: record.pitch,
                roll: record.roll,
                yaw: record.yaw,
                radar_target,
            };
            trace!("{}: {:?}", record.uid, posit_update);
            entity_data
                .position_data
                .as_mut()
                .unwrap()
                .position_updates
                .push(posit_update);
        }
        REC_TYPE_FEATURE_POSITION => {
            let record = FeaturePositionRecord::parse(r)?;

            let feature = FeatureData {
                kind: record.kind,
                lead_uid: record.lead_uid,
                slot: record.slot,
                special_flags: record.special_flags,
                time,
                x: record.x,
                y: record.y,
                z: record.z,
                pitch: record.pitch,
                roll: record.roll,
                yaw: record.yaw,
            };

            if let Some(first_def) = flight.features.get(&record.uid) {
                if *first_def != feature {
                    warn!(
                        "Feature {} defined multiple times with different data! {:?} vs. {:?}",
                        record.uid, first_def, feature
                    );
                }
                return Ok(true);
            }

            trace!("New feature {}: {:?}", record.uid, feature);
            assert!(flight.features.insert(record.uid, feature).is_none());
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
            flight.general_events.push(event);
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
            flight.general_events.push(event);
        }
        REC_TYPE_MOVING_SFX => {
            let record = MovingSoundRecord::parse(r)?;
            let event = GeneralEvent {
                type_byte,
                start: time,
                stop: time + record.ttl,
                kind: record.kind,
                user: record.user,
                flags: record.flags,
                x: record.x,
                y: record.y,
                z: record.z,
                dx: record.dx,
                dy: record.dy,
                dz: record.dz,
                scale: record.scale,
                ..Default::default()
            };
            trace!("Moving sound: {:?}", event);
            flight.general_events.push(event);
        }
        REC_TYPE_SWITCH => {
            let record = SwitchRecord::parse(r)?;
            let entity = flight
                .entities
                .entry(record.uid)
                .or_insert_with(Default::default);

            let payload = EntityEventPayload::SwitchEvent(SwitchEvent {
                switch_number: record.switch_number,
                new_switch_value: record.new_switch_value,
                previous_switch_value: record.previous_switch_value,
            });

            let event = EntityEvent { time, payload };
            trace!("{}: {:?}", record.uid, event);
            entity.events.push(event);
        }
        REC_TYPE_DOF => {
            let record = DofRecord::parse(r)?;
            let entity = flight
                .entities
                .entry(record.uid)
                .or_insert_with(Default::default);

            let payload = EntityEventPayload::DofEvent(DofEvent {
                dof_number: record.dof_number,
                new_dof_value: record.new_dof_value,
                previous_dof_value: record.previous_dof_value,
            });

            let event = EntityEvent { time, payload };
            trace!("{}: {:?}", record.uid, event);
            entity.events.push(event);
        }
        REC_TYPE_TOD_OFFSET => flight.tod_offset = time,
        REC_TYPE_FEATURE_STATUS => {
            let record = FeatureEventRecord::read(r)?;
            let event = FeatureEvent {
                time,
                feature_uid: record.uid,
                new_status: record.new_status,
                previous_status: record.previous_status,
            };
            // Look up the feature by its UID
            if !flight.features.contains_key(&record.uid) {
                warn!("No feature for event {:?}", event);
                return Ok(true);
            }
            trace!("Feature event: {:?}", event);
            flight.feature_events.push(event);
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

struct FeaturePositionRecord {
    kind: i32,
    uid: i32,
    lead_uid: i32,
    slot: i32,
    special_flags: u32,
    x: f32,
    y: f32,
    z: f32,
    yaw: f32,
    pitch: f32,
    roll: f32,
}

impl FeaturePositionRecord {
    fn parse<R: Read>(r: &mut R) -> Result<Self> {
        let kind = read_i32(r)?;
        let uid = read_i32(r)?;
        let lead_uid = read_i32(r)?;
        let slot = read_i32(r)?;
        let special_flags = read_u32(r)?;
        let x = read_f32(r)?;
        let y = read_f32(r)?;
        let z = read_f32(r)?;
        let yaw = read_f32(r)?;
        let pitch = read_f32(r)?;
        let roll = read_f32(r)?;

        Ok(Self {
            kind,
            uid,
            lead_uid,
            slot,
            special_flags,
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
struct MovingSoundRecord {
    kind: i32,
    user: i32,
    flags: u32,
    x: f32,
    y: f32,
    z: f32,
    dx: f32,
    dy: f32,
    dz: f32,
    ttl: f32,
    scale: f32,
}

impl MovingSoundRecord {
    fn parse<R: Read>(r: &mut R) -> Result<Self> {
        let kind = read_i32(r)?;
        let user = read_i32(r)?;
        let flags = read_u32(r)?;
        let x = read_f32(r)?;
        let y = read_f32(r)?;
        let z = read_f32(r)?;
        let dx = read_f32(r)?;
        let dy = read_f32(r)?;
        let dz = read_f32(r)?;
        let ttl = read_f32(r)?;
        let scale = read_f32(r)?;

        Ok(Self {
            kind,
            user,
            flags,
            x,
            y,
            z,
            dx,
            dy,
            dz,
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

#[derive(Debug)]
struct SwitchRecord {
    kind: i32,
    uid: i32,
    switch_number: i32,
    new_switch_value: i32,
    previous_switch_value: i32,
}

impl SwitchRecord {
    fn parse<R: Read>(r: &mut R) -> Result<Self> {
        let kind = read_i32(r)?;
        let uid = read_i32(r)?;
        let switch_number = read_i32(r)?;
        let new_switch_value = read_i32(r)?;
        let previous_switch_value = read_i32(r)?;

        Ok(Self {
            kind,
            uid,
            switch_number,
            new_switch_value,
            previous_switch_value,
        })
    }
}

#[derive(Debug)]
struct DofRecord {
    kind: i32,
    uid: i32,
    dof_number: i32,
    new_dof_value: f32,
    previous_dof_value: f32,
}

impl DofRecord {
    fn parse<R: Read>(r: &mut R) -> Result<Self> {
        let kind = read_i32(r)?;
        let uid = read_i32(r)?;
        let dof_number = read_i32(r)?;
        let new_dof_value = read_f32(r)?;
        let previous_dof_value = read_f32(r)?;

        Ok(Self {
            kind,
            uid,
            dof_number,
            new_dof_value,
            previous_dof_value,
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
