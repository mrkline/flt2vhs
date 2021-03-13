use std::collections::HashMap;
use std::io;
use std::io::prelude::*;
use std::time::Instant;

use anyhow::*;
use log::*;
use rayon::prelude::*;

use crate::flt::Flight;
use crate::print_timing;
use crate::write_primitives::*;

/// The header is 80 bytes long; entities start after.
const ENTITY_OFFSET: u32 = 80;
const ENTITY_SIZE: u32 = 36;
const ENTITY_UPDATE_SIZE: u32 = 41;
const GENERAL_EVENT_SIZE: u32 = 65;
const GENERAL_EVENT_TRAILER_SIZE: u32 = 8;
const FEATURE_EVENT_SIZE: u32 = 16;
const CALLSIGN_RECORD_SIZE: u32 = 20;

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

pub fn write<W: Write>(flight: &Flight, w: &mut W) -> Result<()> {
    let sort_start = Instant::now();
    let mut entity_uids = flight.entities.keys().copied().collect::<Vec<_>>();
    entity_uids.par_sort();
    let mut feature_uids = flight.features.keys().copied().collect::<Vec<_>>();
    feature_uids.par_sort();
    print_timing("Sorting UIDs", &sort_start);

    let write_start = Instant::now();
    let mut counted = CountedWrite::new(w);
    let w = &mut counted;

    let header = Header::new(flight);
    header.write(flight, w)?;
    assert_eq!(w.get_posit(), ENTITY_OFFSET);

    write_entities(flight, &entity_uids, &header, w)?;
    assert_eq!(w.get_posit(), header.feature_offset);

    print_timing("VHS write", &write_start);
    Ok(())
}

#[derive(Debug)]
struct Header {
    entity_count: u32,
    feature_count: u32,
    position_count: u32,
    entity_event_count: u32,
    feature_event_count: u32,
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

        let feature_event_count = flight
            .features
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

        let text_event_offset = feature_event_offset + FEATURE_EVENT_SIZE * feature_event_count;

        let file_length =
            text_event_offset + 4 + CALLSIGN_RECORD_SIZE * flight.callsigns.len() as u32;

        Self {
            entity_count,
            feature_count,
            position_count,
            entity_event_count,
            feature_event_count,
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
        w.write_all(b"EPAT")?;

        // Let's debug print the header. It's fairly short,
        // and the offsets and counts are a good sanity check.

        debug!("File size: {}", self.file_length);
        write_u32(self.file_length, w)?;

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

        // Why does acmi-compiler set this to zero?
        // Don't the callsigns count?
        debug!("Text event count: 0");
        write_u32(0, w)?;

        debug!("Feature event count: {}", self.feature_event_count);
        write_u32(self.feature_event_count, w)?;

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
    sorted_uids: &[i32],
    header: &Header,
    w: &mut W,
) -> Result<()> {
    let mut kind_indexes = HashMap::new();
    let mut position_index = 0;
    let mut event_index = 0;

    for uid in sorted_uids {
        let entity = flight.entities.get(uid).unwrap();
        let data = entity.position_data.as_ref().unwrap();
        write_i32(*uid, w)?;
        write_i32(data.kind, w)?;

        // For some reason (at least per the acmi compiler code), for each entity
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

    Ok(())
}
