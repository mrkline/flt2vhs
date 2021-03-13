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

pub fn write<W: Write>(flight: &Flight, w: &mut W) -> Result<()> {
    let sort_start = Instant::now();
    let mut entity_uids = flight.entities.keys().collect::<Vec<_>>();
    entity_uids.par_sort();
    let mut feature_uids = flight.features.keys().collect::<Vec<_>>();
    feature_uids.par_sort();
    print_timing("Sorting UIDs", &sort_start);

    let header = Header::new(flight);
    header.write(flight, w)?;

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

        let offsets = Self {
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
        };
        offsets
    }

    fn write<W: Write>(&self, flight: &Flight, w: &mut W) -> Result<()> {
        w.write_all(b"EPAT")?;

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
