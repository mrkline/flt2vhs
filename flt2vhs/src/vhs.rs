//! Writes a flight parsed from a `.flt` file into a `.vhs` file

use std::collections::HashMap;
use std::io;
use std::io::prelude::*;

use anyhow::*;
use log::*;
use rayon::prelude::*;

use crate::flt::{self, Flight};
use crate::write_primitives::*;

/// The header is 80 bytes long; entities start after.
const ENTITY_OFFSET: u32 = 80;
const ENTITY_SIZE: u32 = 36;
const ENTITY_UPDATE_SIZE: u32 = 41;
const GENERAL_EVENT_SIZE: u32 = 65;
const GENERAL_EVENT_TRAILER_SIZE: u32 = 8;
const FEATURE_EVENT_SIZE: u32 = 16;
const CALLSIGN_RECORD_SIZE: u32 = 20;

/// We'll want to check our position as we write -
/// keep track ourselves so we don't have to make a bunch of stat() syscalls.
struct CountedWrite<W> {
    inner: W,
    posit: u32, // Welcome to 1998, where files are always < 4 GB.
}

impl<W: Write> CountedWrite<W> {
    fn new(inner: W) -> Self {
        Self { inner, posit: 0 }
    }

    fn get_posit(&self) -> u32 {
        self.posit
    }
}

impl<W: Write> Write for CountedWrite<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let res = self.inner.write(buf);
        if let Ok(count) = res {
            self.posit += count as u32;
        }
        res
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Writes out a VHS flight.
///
/// Will allocate a BufWriter based on expected file size;
/// pass "raw" writer in.
pub fn write<W: Write>(flight: &Flight, w: W) -> Result<()> {
    // We want our FLT -> VHS conversion to be deterministic,
    // so sort the UIDs instead of grabbing them in whatever order they come
    // out of the hash map.
    // Unstable sorts are fine, though, since Unique IDs better be... unique.
    let mut entity_uids = flight.entities.keys().copied().collect::<Vec<_>>();
    entity_uids.par_sort_unstable();
    let mut feature_uids = flight.features.keys().copied().collect::<Vec<_>>();
    feature_uids.par_sort_unstable();

    // Build the header, which will give us an idea of how big the file will be.
    let header = Header::new(flight);

    // Buffer writes up to 100 MB or the file size, whichever is smallest.
    // (100 MB being "a round number that's not too much RAM."; play with this.)
    let buf_size = std::cmp::min(100 * 1024 * 1024, header.file_length);
    let buffered = io::BufWriter::with_capacity(buf_size as usize, w);
    let mut counted = CountedWrite::new(buffered);
    let w = &mut counted;

    header.write(flight, w)?;
    assert_eq!(w.get_posit(), ENTITY_OFFSET);

    let feature_position_offset = write_entities(flight, &entity_uids, &header, w)?;
    assert_eq!(w.get_posit(), header.feature_offset);

    // Some of the feature fields refer to the index of other features.
    // Let's put those in hash map so we can get constant time lookup.
    let mut feature_indexes: HashMap<i32, i32> = HashMap::with_capacity(feature_uids.len());
    for (i, uid) in feature_uids.iter().enumerate() {
        feature_indexes.insert(*uid, i as i32);
    }

    write_features(
        flight,
        &feature_uids,
        &feature_indexes,
        feature_position_offset,
        &header,
        w,
    )?;

    assert_eq!(w.get_posit(), header.position_offset);
    write_entity_positions(flight, &entity_uids, w)?;
    write_feature_positions(flight, &feature_uids, w)?;

    assert_eq!(w.get_posit(), header.entity_event_offset);
    write_entity_events(flight, &entity_uids, w)?;

    assert_eq!(w.get_posit(), header.general_event_offset);
    write_general_events(flight, &header, w)?;

    assert_eq!(w.get_posit(), header.feature_event_offset);
    write_feature_events(flight, &feature_indexes, w)?;

    assert_eq!(w.get_posit(), header.text_event_offset);
    write_callsigns(flight, w)?;

    assert_eq!(w.get_posit(), header.file_length);

    w.flush()?;
    Ok(())
}

/// Lots of sizes and offsets we need to write to the file header,
/// and a couple we don't (but are useful to check against).
///
/// FLT files never exceed 1GB, and the VHS shouldn't exceed much more.
/// It's 1998, and "big" is 32 bits.
#[derive(Debug)]
struct Header {
    entity_count: u32,
    feature_count: u32,
    position_count: u32,
    entity_event_count: u32,
    feature_offset: u32,
    position_offset: u32,
    entity_event_offset: u32,
    general_event_offset: u32,
    general_event_trailer_offset: u32,
    feature_event_offset: u32,
    text_event_offset: u32,
    file_length: u32,
}

impl Header {
    fn new(flight: &Flight) -> Self {
        let entity_count = flight.entities.len() as u32;
        let feature_count = flight.features.len() as u32;
        let entity_position_count = flight
            .entities
            .values()
            .map(|d| {
                d.position_data
                    .as_ref()
                    .expect("No position data")
                    .position_updates
                    .len()
            })
            .sum::<usize>() as u32;
        // Assuming ONE position per feature. Change me if we support multiple.
        let position_count = entity_position_count + flight.features.len() as u32;

        let entity_event_count = flight
            .entities
            .values()
            .map(|d| d.events.len())
            .sum::<usize>() as u32;

        let feature_offset = ENTITY_OFFSET + ENTITY_SIZE * entity_count;

        let position_offset = feature_offset + ENTITY_SIZE * feature_count;

        let entity_event_offset = position_offset + ENTITY_UPDATE_SIZE * position_count;

        let general_event_count = flight.general_events.len() as u32;

        let general_event_offset = entity_event_offset + ENTITY_UPDATE_SIZE * entity_event_count;

        let general_event_trailer_offset =
            general_event_offset + GENERAL_EVENT_SIZE * general_event_count;

        let feature_event_offset =
            general_event_trailer_offset + GENERAL_EVENT_TRAILER_SIZE * general_event_count;

        let text_event_offset =
            feature_event_offset + FEATURE_EVENT_SIZE * flight.feature_events.len() as u32;

        let file_length =
            text_event_offset + 4 + CALLSIGN_RECORD_SIZE * flight.callsigns.len() as u32;

        Self {
            entity_count,
            feature_count,
            position_count,
            entity_event_count,
            feature_offset,
            position_offset,
            entity_event_offset,
            general_event_offset,
            general_event_trailer_offset,
            feature_event_offset,
            text_event_offset,
            file_length,
        }
    }

    fn write<W: Write>(&self, flight: &Flight, w: &mut W) -> Result<()> {
        // The magic bytes: "TAPE", but little-endian.
        w.write_all(b"EPAT")?;

        // Let's debug print the header. It's fairly short,
        // and the offsets and counts are a good sanity check.

        debug!("File size: {}", self.file_length);
        // Weird: acmi-compiler sets this to the text event offset,
        // not the actual length, and it's just doing what FreeFalcon
        // (so presumably F4 and BMS) do.
        // TacView throws a fit if this isn't the case.
        write_u32(self.text_event_offset, w)?;

        debug!("Entity count: {}", self.entity_count);
        write_u32(self.entity_count, w)?;

        debug!("Feature count: {}", self.feature_count);
        write_u32(self.feature_count, w)?;

        debug!("Entity offset: {}", ENTITY_OFFSET);
        write_u32(ENTITY_OFFSET, w)?;

        debug!("Feature offset: {}", self.feature_offset);
        write_u32(self.feature_offset, w)?;

        debug!("Position count: {}", self.position_count);
        write_u32(self.position_count, w)?;

        debug!("Position offset: {}", self.position_offset);
        write_u32(self.position_offset, w)?;

        debug!("Entity event offset: {}", self.entity_event_offset);
        write_u32(self.entity_event_offset, w)?;

        debug!("General event offset: {}", self.general_event_offset);
        write_u32(self.general_event_offset, w)?;

        debug!(
            "General event trailer offset: {}",
            self.general_event_trailer_offset
        );
        write_u32(self.general_event_trailer_offset, w)?;

        debug!("Text event offset: {}", self.text_event_offset);
        write_u32(self.text_event_offset, w)?;

        debug!("Feature event offset: {}", self.feature_event_offset);
        write_u32(self.feature_event_offset, w)?;

        debug!("General event count: {}", flight.general_events.len());
        write_u32(flight.general_events.len() as u32, w)?;

        debug!("Entity event count: {}", self.entity_event_count);
        write_u32(self.entity_event_count, w)?;

        // Callsigns aren't text events - they're a separate thing that
        // don't seem to get saved anymore. Looking at the FreeFalcon code,
        // seems like they were pulled from the game state and not the .FLT.
        debug!("Text event count: 0");
        write_u32(0, w)?;

        debug!("Feature event count: {}", flight.feature_events.len());
        write_u32(flight.feature_events.len() as u32, w)?;

        debug!("Start time: {}", flight.start_time);
        write_f32(flight.start_time, w)?;

        let total_time = flight.end_time - flight.start_time;
        debug!("Total play time: {}", total_time);
        write_f32(total_time, w)?;

        debug!("Time of day offset: {}", flight.tod_offset);
        write_f32(flight.tod_offset, w)?;

        Ok(())
    }
}

/// Write the out the list of entities.
///
/// Entities and features (things that don't move) share the same on-disk format,
/// so some fields aren't used. Each contains:
///
/// - A UID
///
/// - A type (or "kind" here since it's a keyword in Rust :P )
///
/// - An index relative to all other entities of the same kind.
///
/// - Flags indicating the category of entity (plane, missile, chaff, flare, etc.)
///
/// - Lead index, slot, and special flags: All unused entities, used by features.
///
/// - The offset of the first position update for the entity, which acts as
///   the head of a doubly-linked list of positions for it.
///
/// - The offset of the first event for the entity, which acts as the head of
///   a second doubly-linked list (this one of events).
fn write_entities<W: Write>(
    flight: &Flight,
    entity_uids: &[i32],
    header: &Header,
    w: &mut W,
) -> Result<u32> {
    let mut kind_indexes = HashMap::new();
    let mut position_index = 0;
    let mut event_index = 0;

    for uid in entity_uids {
        let entity = flight.entities.get(uid).unwrap();
        let data = entity.position_data.as_ref().unwrap();
        write_i32(*uid, w)?;
        write_i32(data.kind, w)?;

        // For some reason (at least per the acmi-compiler code), for each entity
        // we store its index (starting at 1!?) out of all entities of the same kind.
        let kind_index = kind_indexes.entry(data.kind).or_insert(1);
        write_i32(*kind_index, w)?;
        *kind_index += 1;

        write_u32(data.flags, w)?;
        // Lead index, slot, and special flags:
        // all 0 for entities. Meaningful for features.
        write_i32(0, w)?;
        write_i32(0, w)?;
        write_u32(0, w)?;

        // Every entity should have at least one position,
        // and we've screwed something up if we get here without one.
        assert!(!data.position_updates.is_empty());
        let first_position_offset = header.position_offset + ENTITY_UPDATE_SIZE * position_index;
        assert!(first_position_offset >= header.position_offset);
        assert!(first_position_offset < header.entity_event_offset);

        write_u32(first_position_offset, w)?;

        let first_event_offset = if entity.events.is_empty() {
            0
        } else {
            header.entity_event_offset + ENTITY_UPDATE_SIZE * event_index
        };
        write_u32(first_event_offset, w)?;

        position_index += data.position_updates.len() as u32;
        event_index += entity.events.len() as u32;
    }

    Ok(header.position_offset + ENTITY_UPDATE_SIZE * position_index)
}

/// Write out the list of features - see [`write_entities()`](write_entities),
/// since features share the same format in the VHS file.
fn write_features<W: Write>(
    flight: &Flight,
    feature_uids: &[i32],
    feature_indexes: &HashMap<i32, i32>,
    feature_position_offset: u32,
    header: &Header,
    w: &mut W,
) -> Result<()> {
    for (position_index, uid) in feature_uids.iter().enumerate() {
        let feature = flight.features.get(uid).unwrap();
        write_i32(*uid, w)?;
        write_i32(feature.kind, w)?;

        // Features don't play the same "type index" game entities do.
        write_i32(0, w)?;

        write_u32(flt::ENTITY_FLAG_FEATURE, w)?;

        // Lead index
        write_i32(*feature_indexes.get(&feature.lead_uid).unwrap_or(&-1), w)?;

        write_i32(feature.slot, w)?;
        write_u32(feature.special_flags, w)?;

        let position_offset =
            feature_position_offset + ENTITY_UPDATE_SIZE * (position_index as u32);
        assert!(position_offset >= feature_position_offset);
        assert!(position_offset < header.entity_event_offset);
        write_u32(position_offset, w)?;

        // Since feature events are stored separately,
        // first event offset is apparently always zero.
        write_u32(0, w)?;
    }

    Ok(())
}

fn write_entity_positions<W: Write>(
    flight: &Flight,
    entity_uids: &[i32],
    w: &mut CountedWrite<W>,
) -> Result<()> {
    // Radar targets need to be converted from UIDs to entity indexes.
    let mut entity_indexes: HashMap<i32, i32> = HashMap::with_capacity(entity_uids.len());
    for (i, uid) in entity_uids.iter().enumerate() {
        entity_indexes.insert(*uid, i as i32);
    }

    for uid in entity_uids {
        let entity = flight.entities.get(uid).unwrap();
        let data = entity.position_data.as_ref().unwrap();

        let mut previous_offset = 0u32;
        let mut posits = data.position_updates.iter().peekable();

        while let Some(new_posit) = posits.next() {
            let current_offset = w.get_posit();
            write_f32(new_posit.time, w)?;
            // Updates are unions of position updates,
            // switch updates, and DOF updates.
            // The next byte is the union's tag/descriminant.
            write_u8(0, w)?;
            write_f32(new_posit.x, w)?;
            write_f32(new_posit.y, w)?;
            write_f32(new_posit.z, w)?;
            write_f32(new_posit.pitch, w)?;
            write_f32(new_posit.roll, w)?;
            write_f32(new_posit.yaw, w)?;
            // Radar target index
            write_i32(
                *entity_indexes.get(&new_posit.radar_target).unwrap_or(&-1),
                w,
            )?;

            // What's nice about having all an entity's position updates in
            // a contiguous lists is that we can write them out contiguously,
            // making the doubly-linked list bookkeeping trivial.
            let next_offset = if posits.peek().is_some() {
                current_offset + ENTITY_UPDATE_SIZE
            } else {
                0
            };
            write_u32(next_offset, w)?;
            write_u32(previous_offset, w)?;
            previous_offset = current_offset;
        }
    }
    Ok(())
}

fn write_feature_positions<W: Write>(
    flight: &Flight,
    entity_uids: &[i32],
    w: &mut W,
) -> Result<()> {
    for uid in entity_uids {
        let feature = flight.features.get(uid).unwrap();

        write_f32(feature.time, w)?;
        // Updates are unions of position updates,
        // switch updates, and DOF updates.
        // The next byte is the union's tag/discriminant.
        write_u8(0, w)?;
        write_f32(feature.x, w)?;
        write_f32(feature.y, w)?;
        write_f32(feature.z, w)?;
        write_f32(feature.pitch, w)?;
        write_f32(feature.roll, w)?;
        write_f32(feature.yaw, w)?;
        // Radar target
        write_i32(-1, w)?;
        // No previous or next positions
        write_u32(0, w)?;
        write_u32(0, w)?;
    }
    Ok(())
}

fn write_entity_events<W: Write>(
    flight: &Flight,
    entity_uids: &[i32],
    w: &mut CountedWrite<W>,
) -> Result<()> {
    for uid in entity_uids {
        let entity = flight.entities.get(uid).unwrap();
        let mut events = entity.events.iter().peekable();

        let mut previous_offset = 0u32;

        while let Some(event) = events.next() {
            let current_offset = w.get_posit();
            write_f32(event.time, w)?;
            // Updates are unions of position updates,
            // switch updates, and DOF updates.
            // The next byte is the union's tag/discriminant.
            match event.payload {
                flt::EntityEventPayload::SwitchEvent(switch) => {
                    write_u8(1, w)?;
                    write_i32(switch.switch_number, w)?;
                    write_i32(switch.new_switch_value, w)?;
                    write_i32(switch.previous_switch_value, w)?;
                }
                flt::EntityEventPayload::DofEvent(dof) => {
                    write_u8(2, w)?;
                    write_i32(dof.dof_number, w)?;
                    write_f32(dof.new_dof_value, w)?;
                    write_f32(dof.previous_dof_value, w)?;
                }
            };
            // Unused space, taken up by the position update in the union
            write_u32(0, w)?;
            write_u32(0, w)?;
            write_u32(0, w)?;
            write_u32(0, w)?;

            // What's nice about having all an entity's position updates in
            // a contiguous lists is that we can write them out contiguously,
            // making the doubly-linked list bookkeeping trivial.
            let next_offset = if events.peek().is_some() {
                current_offset + ENTITY_UPDATE_SIZE
            } else {
                0
            };
            write_u32(next_offset, w)?;
            write_u32(previous_offset, w)?;
            previous_offset = current_offset;
        }
    }
    Ok(())
}

struct GeneralEventTrailer {
    stop: f32,
    index: u32,
}

fn write_general_events<W: Write>(
    flight: &Flight,
    header: &Header,
    w: &mut CountedWrite<W>,
) -> Result<()> {
    let mut trailers = Vec::with_capacity(flight.general_events.len());

    for (i, event) in flight.general_events.iter().enumerate() {
        let i = i as u32;
        trailers.push(GeneralEventTrailer {
            stop: event.stop,
            index: i,
        });

        write_u8(event.type_byte, w)?;
        write_u32(i, w)?;
        write_f32(event.start, w)?;
        write_f32(event.stop, w)?;
        write_i32(event.kind, w)?;
        write_i32(event.user, w)?;
        write_u32(event.flags, w)?;
        write_f32(event.scale, w)?;
        write_f32(event.x, w)?;
        write_f32(event.y, w)?;
        write_f32(event.z, w)?;
        write_f32(event.dx, w)?;
        write_f32(event.dy, w)?;
        write_f32(event.dz, w)?;
        write_f32(event.roll, w)?;
        write_f32(event.pitch, w)?;
        write_f32(event.yaw, w)?;
    }

    // A list of "trailers" follows the event list, sorted chronologically.
    assert_eq!(w.get_posit(), header.general_event_trailer_offset);
    trailers.par_sort_by(|a, b| a.stop.partial_cmp(&b.stop).expect("Nooo, not NaNs!"));
    for trailer in trailers {
        write_f32(trailer.stop, w)?;
        write_u32(trailer.index, w)?;
    }

    Ok(())
}

fn write_feature_events<W: Write>(
    flight: &Flight,
    feature_indexes: &HashMap<i32, i32>,
    w: &mut CountedWrite<W>,
) -> Result<()> {
    for event in &flight.feature_events {
        let index = *feature_indexes
            .get(&event.feature_uid)
            .expect("Feature event with no feature");
        write_f32(event.time, w)?;
        write_i32(index, w)?;
        write_i32(event.new_status, w)?;
        write_i32(event.previous_status, w)?;
    }

    Ok(())
}

fn write_callsigns<W: Write>(flight: &Flight, w: &mut W) -> Result<()> {
    write_u32(flight.callsigns.len() as u32, w)?;
    for callsign in &flight.callsigns {
        w.write_all(&callsign.label)?;
        write_i32(callsign.team_color, w)?;
    }
    Ok(())
}
