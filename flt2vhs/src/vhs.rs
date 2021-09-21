//! Writes a flight parsed from a `.flt` file into a `.vhs` file

use std::io;
use std::io::prelude::*;

use anyhow::*;
use log::*;
use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::flt::{self, Flight};
use crate::primitives::*;

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
    fn new(inner: W, initial_posit: u32) -> Self {
        Self {
            inner,
            posit: initial_posit,
        }
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

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd)]
enum IdType {
    Entity,
    Feature,
}

// Maps an ID from its original in-FLT value to the one we'll write to disk.
#[derive(Debug, Copy, Clone)]
struct IdRemap {
    original: i32,
    new: i32,
}

/// Welcome to whose VHS is it anyways, where the IDs are made up
/// and the order DOES matter.
///
/// IDs are used to index into the callsign table (containing name and team info)
/// at the end of the file, and the ones that come from BMS are _very_ sparse.
/// As fun as it is to write a bunch of zeroes in back, let's cook up our own
/// IDs instead.
#[derive(Debug, Clone)]
struct IdMapping {
    entities: Vec<IdRemap>,
    features: Vec<IdRemap>,
    callsign_ids: Vec<i32>,
}

impl IdMapping {
    fn new(flight: &flt::Flight) -> Self {
        // Make a list of every ID...
        let mut all_ids = flight
            .entities
            .keys()
            .map(|eid| (*eid, IdType::Entity))
            .chain(flight.features.keys().map(|fid| (*fid, IdType::Feature)))
            .collect::<Vec<(i32, IdType)>>();

        // Sort it so that IDs _with_ callsign info come first...
        // Unstable sorts are fine - each entry should always be a unique (ID, type)
        // tuple, even in the sad scenario where a feature and an entity share IDs.
        all_ids.par_sort_unstable_by(|(left, left_kind), (right, right_kind)| {
            use std::cmp::Ordering;
            match (
                flight.callsigns.contains_key(left),
                flight.callsigns.contains_key(right),
            ) {
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                // Sorting IDs (instead of grabbing them in whatever order they come
                // out of the hash map) seems to put the player first,
                // and it's cheap compared to everything else we're doing.
                _ => {
                    // Since we're sorting anyways,
                    // partition entities and features to be nicer to the branch
                    // predictor as we filter through this below.
                    match left_kind.cmp(right_kind) {
                        Ordering::Equal => left.cmp(right),
                        different => different,
                    }
                }
            }
        });

        // Tada! If we replace each ID with its index in `all_ids`,
        // ones with callsign info are [0..flight.callsigns.len()].
        // The callsign list is now as compact as possible.
        let callsign_bound = all_ids.partition_point(|(id, _)| flight.callsigns.contains_key(id));
        let callsign_ids = all_ids
            .iter()
            .map(|(id, _)| *id)
            .take(callsign_bound)
            .collect();

        let entities = all_ids
            .iter()
            .enumerate()
            .filter_map(|(i, (id, kind))| {
                (*kind == IdType::Entity).then(|| IdRemap {
                    original: *id,
                    new: i as i32,
                })
            })
            .collect();

        let features = all_ids
            .iter()
            .enumerate()
            .filter_map(|(i, (id, kind))| {
                (*kind == IdType::Feature).then(|| IdRemap {
                    original: *id,
                    new: i as i32,
                })
            })
            .collect();

        Self {
            entities,
            features,
            callsign_ids,
        }
    }
}

/// Writes out a VHS flight.
///
/// Will allocate a BufWriter based on expected file size;
/// pass "raw" writer in.
/// Returns the number of bytes written on success.
pub fn write(flight: &Flight, fh: std::fs::File) -> Result<u32> {
    let id_map = IdMapping::new(flight);

    // Build the header, which will give us an idea of how big the file will be.
    let header = Header::new(flight, id_map.callsign_ids.len());

    // Set the file length and map it for writing.
    fh.set_len(header.file_length as u64)
        .context("Couldn't grow output file")?;
    let mut mapped =
        unsafe { memmap::MmapMut::map_mut(&fh) }.context("Couldn't memory map output file")?;

    // We know in advance how large each section of the file will be - we did
    // that math to build the header. Slice the file mapping into mutable slices
    // for each section, which we can write out in parallel below.
    let (mut header_slice, rest) = mapped.split_at_mut(ENTITY_OFFSET as usize);
    let (mut entity_slice, rest) =
        rest.split_at_mut((header.feature_offset - ENTITY_OFFSET) as usize);
    let (mut feature_slice, rest) =
        rest.split_at_mut((header.position_offset - header.feature_offset) as usize);
    let (position_slice, rest) =
        rest.split_at_mut((header.entity_event_offset - header.position_offset) as usize);
    let (entity_events_slice, rest) =
        rest.split_at_mut((header.general_event_offset - header.entity_event_offset) as usize);
    let (mut general_events_slice, rest) =
        rest.split_at_mut((header.feature_event_offset - header.general_event_offset) as usize);
    let (mut feature_events_slice, mut callsigns_slice) =
        rest.split_at_mut((header.text_event_offset - header.feature_event_offset) as usize);

    // Some of the feature fields refer to the index of other features.
    // Let's put those in hash map so we can get constant time lookup.
    let mut feature_indexes: FxHashMap<i32, i32> =
        FxHashMap::with_capacity_and_hasher(flight.features.len(), Default::default());
    for (i, id) in id_map.features.iter().map(|m| m.original).enumerate() {
        feature_indexes.insert(id, i as i32);
    }

    // Parallelize ALL the writes!
    // We'll unwrap the results of any write failures because:
    //
    // 1. Rayon's scope doesn't have good tools to propagate errors back.
    //
    // 2. We've allocated the whole file already and mapped it to memory.
    //    Something truly bizarre is happening if a write fails.
    rayon::scope(|s| {
        s.spawn(|_| {
            header
                .write(flight, &mut header_slice)
                .expect("Header write failed")
        });

        s.spawn(|_| {
            let feature_position_offset =
                write_entities(flight, &id_map.entities, &header, &mut entity_slice)
                    .expect("Entity write failed");

            write_features(
                flight,
                &id_map.features,
                &feature_indexes,
                feature_position_offset,
                &header,
                &mut feature_slice,
            )
            .expect("Feature write failed");
        });

        s.spawn(|_| {
            let mut position_write = CountedWrite::new(position_slice, header.position_offset);
            write_entity_positions(flight, &id_map.entities, &mut position_write)
                .expect("Entity positions write failed");
            write_feature_positions(flight, &id_map.features, &mut position_write)
                .expect("Feature position write failed");
            assert_eq!(position_write.get_posit(), header.entity_event_offset);
        });

        s.spawn(|_| {
            let mut entity_events_write =
                CountedWrite::new(entity_events_slice, header.entity_event_offset);
            write_entity_events(flight, &id_map.entities, &mut entity_events_write)
                .expect("Entity events write failed");
            assert_eq!(entity_events_write.get_posit(), header.general_event_offset);
        });

        s.spawn(|_| {
            write_general_events(flight, &mut general_events_slice)
                .expect("General events write failed")
        });

        s.spawn(|_| {
            write_feature_events(flight, &feature_indexes, &mut feature_events_slice)
                .expect("Feature events write failed")
        });

        s.spawn(|_| {
            write_callsigns(flight, &id_map.callsign_ids, &mut callsigns_slice)
                .expect("Callsigns write failed")
        });
    });

    mapped.flush()?;

    Ok(header.file_length)
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
    fn new(flight: &Flight, num_callsigns: usize) -> Self {
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

        let callsign_array_len = num_callsigns as u32;

        let file_length = text_event_offset + 4 + CALLSIGN_RECORD_SIZE * callsign_array_len;

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
        // seems like they were pulled from the game state and not the FLT.
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
    entity_mapping: &[IdRemap],
    header: &Header,
    w: &mut W,
) -> Result<u32> {
    let mut kind_indexes = FxHashMap::default();
    let mut position_index = 0;
    let mut event_index = 0;

    for (id, entity) in entity_mapping
        .iter()
        .map(|remap| (remap.new, &flight.entities[&remap.original]))
    {
        let data = entity.position_data.as_ref().unwrap();
        write_i32(id, w)?;
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
    feature_mapping: &[IdRemap],
    feature_indexes: &FxHashMap<i32, i32>,
    feature_position_offset: u32,
    header: &Header,
    w: &mut W,
) -> Result<()> {
    for (position_index, (id, feature)) in feature_mapping
        .iter()
        .map(|remap| (remap.new, &flight.features[&remap.original]))
        .enumerate()
    {
        write_i32(id, w)?;
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
    entity_mapping: &[IdRemap],
    w: &mut CountedWrite<W>,
) -> Result<()> {
    // Radar targets need to be converted from UIDs to entity indexes.
    let mut entity_indexes: FxHashMap<i32, i32> =
        FxHashMap::with_capacity_and_hasher(flight.entities.len(), Default::default());
    for (i, id) in entity_mapping.iter().map(|m| m.original).enumerate() {
        entity_indexes.insert(id, i as i32);
    }

    for entity in entity_mapping
        .iter()
        .map(|remap| &flight.entities[&remap.original])
    {
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
    feature_mapping: &[IdRemap],
    w: &mut W,
) -> Result<()> {
    for feature in feature_mapping
        .iter()
        .map(|remap| &flight.features[&remap.original])
    {
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
    entity_mapping: &[IdRemap],
    w: &mut CountedWrite<W>,
) -> Result<()> {
    for entity in entity_mapping
        .iter()
        .map(|remap| &flight.entities[&remap.original])
    {
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

fn write_general_events<W: Write>(flight: &Flight, w: &mut W) -> Result<()> {
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
    trailers.par_sort_by(|a, b| a.stop.partial_cmp(&b.stop).expect("Nooo, not NaNs!"));
    for trailer in trailers {
        write_f32(trailer.stop, w)?;
        write_u32(trailer.index, w)?;
    }

    Ok(())
}

fn write_feature_events<W: Write>(
    flight: &Flight,
    feature_indexes: &FxHashMap<i32, i32>,
    w: &mut W,
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

fn write_callsigns<W: Write>(flight: &Flight, callsign_ids: &[i32], w: &mut W) -> Result<()> {
    if callsign_ids.is_empty() {
        warn!("No callsigns to save!");
        write_u32(0, w)?;
        return Ok(());
    }

    write_u32(callsign_ids.len() as u32, w)?;
    for callsign in callsign_ids.iter().map(|id| &flight.callsigns[id]) {
        w.write_all(&callsign.label)?;
        write_i32(callsign.team_color, w)?;
    }
    Ok(())
}
