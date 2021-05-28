#![allow(clippy::float_cmp)]
//! Parses info we need from a `.flt` file

use std::{io, io::prelude::*, path::Path, time::Instant};

use anyhow::*;
use fnv::{FnvHashMap, FnvHashSet};
use log::*;

use crate::primitives::*;

/// Information parsed from a .flt file, needed to make a .vhs file
#[derive(Debug, Clone, Default)]
pub struct Flight {
    /// True if something went wrong reading the `.flt`
    /// (ran out of bytes), bad reads, etc.
    ///
    /// We'll warn the user but still do the best with what we have.
    pub corrupted: bool,

    /// Time of day offset
    pub tod_offset: f32,

    /// Recording start time
    pub start_time: f32,

    /// Recording end time
    pub end_time: f32,

    /// Map entity & feature UIDs to callsigns (16 byte blocks for strings) and faction colors.
    ///
    /// Use an ordered map to quickly inflate it back to an array on VHS write.
    pub callsigns: FnvHashMap<i32, CallsignRecord>,

    /// A map of unique IDs for entities (all moving objects in game)
    /// to their position updates and events.
    pub entities: FnvHashMap<i32, EntityData>,

    /// A map of unique IDs for all features (static objects)
    /// and their positions.
    pub features: FnvHashMap<i32, FeatureData>,

    /// "General" events not associated with any particular entities
    pub general_events: Vec<GeneralEvent>,

    /// Feature events.
    ///
    /// Feature events aren't mapped to their features' UIDs in FeatureData
    /// (like entity events are in EntityData).
    ///
    /// They need to be written chronologically
    /// (or at least in the order they were in the `.flt` file),
    /// so we'd have to flatten it all back out into a vector anyways.
    pub feature_events: Vec<FeatureEvent>,
}

impl Flight {
    pub fn parse<R: Read>(mut r: R) -> Self {
        let mut flight = Self {
            start_time: f32::NEG_INFINITY,
            end_time: f32::NEG_INFINITY,
            ..Default::default()
        };

        // A .flt is a flat stream of events of different types,
        // discriminated by a leading byte.
        loop {
            match read_record(&mut flight, &mut r) {
                Ok(true) => { /* continue */ }
                Ok(false) => break, // EOF
                Err(e) => {
                    warn_on(e);
                    flight.corrupted = true;
                    break;
                }
            }
        }

        // Some entities get events, but never any position updates to place
        // them in the world and provide their other needed state.
        // Very strange, but let's throw them out since we can't do anything
        // with events for an entity that was never defined.
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

    pub fn merge(
        &mut self,
        next_flight: &Flight,
        previous_flight_path: &Path,
        next_flight_path: &Path,
    ) -> bool {
        let previous_flight_path = previous_flight_path.display();
        let next_flight_path = next_flight_path.display();

        debug!(
            "Considering if {} and {} should be merged...",
            previous_flight_path, next_flight_path
        );

        if self.corrupted {
            debug!("...no, {} is corrupted", previous_flight_path);
            return false;
        }

        if self.tod_offset != next_flight.tod_offset {
            debug!(
                "...no, {} and {} are at different times of day",
                previous_flight_path, next_flight_path
            );
        }

        let dt = next_flight.start_time - self.end_time;
        if dt > 1.0 {
            debug!(
                "...no, {} and {} are more than a second apart",
                previous_flight_path, next_flight_path
            );
            return false;
        }

        debug!("...yes!");
        info!("Merging {} into {}", next_flight_path, previous_flight_path);
        let start_time = Instant::now();

        // If we're adding corrupted data to uncorrupted, propagate that.
        self.corrupted |= next_flight.corrupted;
        self.end_time = next_flight.end_time;

        // An unused ID in self that we can use for new entities in next_flight
        let mut unique_id = std::cmp::max(
            *self.entities.keys().max().unwrap(),
            *self.features.keys().max().unwrap(),
        ) + 1;

        self.merge_entities(next_flight, &mut unique_id);
        self.merge_features(next_flight, &mut unique_id);

        self.general_events
            .extend_from_slice(&next_flight.general_events);

        crate::print_timing("Merge", &start_time);
        true
    }

    fn merge_entities(self: &mut Flight, next_flight: &Flight, unique_id: &mut i32) {
        let starting_uid = *unique_id;

        let mut used_previous_entities: FnvHashSet<i32> =
            FnvHashSet::with_capacity_and_hasher(self.entities.len(), Default::default());
        let mut next_to_previous_ids: FnvHashMap<i32, i32> =
            FnvHashMap::with_capacity_and_hasher(next_flight.entities.len(), Default::default());

        for (next_id, next_entity) in &next_flight.entities {
            let mut closest_entity: Option<i32> = None;
            let mut closest_distance = f32::INFINITY;

            let next_data = next_entity.position_data.as_ref().unwrap();

            for (previous_id, previous_entity) in &self.entities {
                let previous_data = previous_entity.position_data.as_ref().unwrap();

                // Entities don't (shouldn't!) change type from file to file.
                // Skip over ones that don't match.
                if next_data.kind != previous_data.kind {
                    continue;
                }

                // Don't consider entities we've already matched up.
                //
                // Along with trying to get some perf wins (are we? profile!),
                // we want to make sure that entities right on top of each other
                // (happens to ground units occasionally) get some 1 to 1 mapping.
                if used_previous_entities.contains(previous_id) {
                    continue;
                }

                let next_position = next_data.position_updates.first().unwrap();
                let previous_position = previous_data.position_updates.last().unwrap();

                let distance = ((next_position.x - previous_position.x).powi(2)
                    + (next_position.y - previous_position.y).powi(2)
                    + (next_position.z - previous_position.z).powi(2))
                .sqrt();

                if distance < closest_distance {
                    closest_entity = Some(*previous_id);
                    closest_distance = distance;
                }
                if distance == 0.0 {
                    // We're not gonna do any better.
                    break;
                }
            }

            if closest_distance < 5280.0 {
                let closest_id = closest_entity.unwrap();

                // We found an entity of the same kind nearby (< 1 mi) in self.
                assert!(used_previous_entities.insert(closest_id));
                assert!(next_to_previous_ids.insert(*next_id, closest_id).is_none());
            } else {
                // We couldn't find anything close in self.
                // Create a brand new entity there.
                assert!(next_to_previous_ids.insert(*next_id, *unique_id).is_none());

                // Add a callsign record.
                if let Some(callsign) = next_flight.callsigns.get(next_id) {
                    assert!(self.callsigns.insert(*unique_id, *callsign).is_none());
                }

                *unique_id += 1;
            }
        }

        // Now that we know how all entities in next_flight map to self,
        // fix up their radar targets and copy them over.
        assert_eq!(next_to_previous_ids.len(), next_flight.entities.len());
        for (next_id, next_entity) in &next_flight.entities {
            let previous_id = next_to_previous_ids[next_id];

            let from = next_entity.position_data.as_ref().unwrap();

            let to = &mut self
                .entities
                .entry(previous_id)
                .or_insert(EntityData {
                    position_data: Some(EntityPositionData {
                        position_updates: Vec::with_capacity(from.position_updates.len()),
                        ..*from
                    }),
                    events: next_entity.events.clone(),
                })
                .position_data
                .as_mut()
                .unwrap()
                .position_updates;

            let from = &from.position_updates;
            to.reserve(from.len());

            for position in from {
                let radar_target = *next_to_previous_ids
                    .get(&position.radar_target)
                    .unwrap_or(&-1);

                to.push(EntityPositionUpdate {
                    radar_target,
                    ..*position
                });
            }
        }

        let new_entities = (*unique_id - starting_uid) as usize;
        debug!(
            "{} new entities, {} merged",
            new_entities,
            next_flight.entities.len() - new_entities
        );
    }

    fn merge_features(self: &mut Flight, next_flight: &Flight, unique_id: &mut i32) {
        let starting_uid = *unique_id;

        let mut next_to_previous_ids: FnvHashMap<i32, i32> =
            FnvHashMap::with_capacity_and_hasher(next_flight.features.len(), Default::default());

        for (next_id, next_feature) in &next_flight.features {
            let mut matching_previous = None;

            for (previous_id, previous_feature) in &self.features {
                if next_feature.kind != previous_feature.kind
                    || next_feature.slot != previous_feature.slot
                    || next_feature.special_flags != previous_feature.special_flags
                    || next_feature.x != previous_feature.x
                    || next_feature.y != previous_feature.y
                    || next_feature.z != previous_feature.z
                    || next_feature.pitch != previous_feature.pitch
                    || next_feature.roll != previous_feature.roll
                    || next_feature.yaw != previous_feature.yaw
                {
                    continue;
                }

                // Everything but the time and lead IDs
                // (which can change between files) matches.
                // It's probably the same thing.
                // (Unlike entities, we don't really care about a 1 to 1 mapping
                // since features don't move.)
                matching_previous = Some(*previous_id);
                break;
            }
            if let Some(previous_id) = matching_previous {
                // If the feature already existed in self,
                // no need to do anything to it.
                next_to_previous_ids.insert(*next_id, previous_id);
            } else {
                // If the feature is new to next_flight,
                // create a new ID for it
                next_to_previous_ids.insert(*next_id, *unique_id);

                // Add a callsign record.
                if let Some(callsign) = next_flight.callsigns.get(next_id) {
                    assert!(self.callsigns.insert(*unique_id, *callsign).is_none());
                }
                *unique_id += 1;
            }
        }

        // Now that we know how all features in next_flight map to self,
        // fix up their parent IDs and copy them over.
        assert_eq!(next_to_previous_ids.len(), next_flight.features.len());
        for (next_id, next_feature) in &next_flight.features {
            let previous_id = next_to_previous_ids[next_id];
            if previous_id < starting_uid {
                continue; // It's already in self.
            }

            let to_copy = FeatureData {
                lead_uid: next_to_previous_ids[next_id],
                ..*next_feature
            };
            assert!(self.features.insert(previous_id, to_copy).is_none());
        }

        // While we have next_to_previous_ids,
        // let's copy over all the feature events, fixing up their IDs.
        self.feature_events
            .reserve(next_flight.feature_events.len());
        for feature_event in &next_flight.feature_events {
            self.feature_events.push(FeatureEvent {
                feature_uid: next_to_previous_ids[&feature_event.feature_uid],
                ..*feature_event
            });
        }

        let new_features = (*unique_id - starting_uid) as usize;
        debug!(
            "{} new features, {} merged",
            new_features,
            next_flight.features.len() - new_features
        );
    }
}

const ENTITY_FLAG_MISSILE: u32 = 0x00000001;
// Used by the vhs module when writing features.
// All entities store theirs, so we don't need to export the other flags.
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

    /// UID of the current radar target, or -1 if the entity doesn't have one.
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
                    trace!(
                        "Position update for entity {} switched kinds from {} to {}",
                        record.uid,
                        posit_data.kind,
                        record.kind
                    );
                }

                if posit_data.flags != flags {
                    trace!(
                        "Position update for entity {} switched flags from {} to {}",
                        record.uid,
                        posit_data.flags,
                        flags
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
                    trace!(
                        "Feature {} defined multiple times ({:?} -> {:?})! Ignoring subsequent ones",
                        record.uid,
                        first_def,
                        feature
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
                stop: time + 5.0, // Should we make this configurable?
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
                trace!("No feature for {:?}", event);
                return Ok(true);
            }
            trace!("Feature event: {:?}", event);
            flight.feature_events.push(event);
        }
        REC_TYPE_CALLSIGN_LIST => {
            if !flight.callsigns.is_empty() {
                warn!("Multiple callsign lists found, using the latest");
                flight.callsigns.clear();
            }

            // Callsign data is a sparse array where indexing by ID
            // gives you name and faction.
            let callsign_array = parse_callsigns(r)?;

            // Callsigns are always written at the end,
            // so we can safely assume entity and feature maps are filled out.
            for entity_key in flight.entities.keys() {
                let index = *entity_key as usize;
                if index >= callsign_array.len() {
                    continue;
                }
                let callsign = callsign_array[index];
                if callsign == CallsignRecord::default() {
                    continue;
                }
                assert!(flight.callsigns.insert(*entity_key, callsign).is_none());
            }

            for feature_key in flight.features.keys() {
                let index = *feature_key as usize;
                if index >= callsign_array.len() {
                    continue;
                }
                let callsign = callsign_array[index];
                if callsign == CallsignRecord::default() {
                    continue;
                }
                assert!(flight.callsigns.insert(*feature_key, callsign).is_none());
            }
        }
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

// Records read out of the `.flt` file.

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

    let mut callsigns = Vec::with_capacity(callsign_count as usize);

    for _ in 0..callsign_count {
        callsigns.push(CallsignRecord::read(r)?);
    }
    Ok(callsigns)
}

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq)]
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
