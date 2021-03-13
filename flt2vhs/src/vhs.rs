use std::io::prelude::*;
use std::time::Instant;

use anyhow::*;
use log::*;
use rayon::prelude::*;

use crate::flt::Flight;
use crate::print_timing;
use crate::write_primitives::*;

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
}

impl Header {
    fn new(flight: &Flight) -> Self {
        let entity_count = flight.entities.keys().count() as u32;
        let feature_count = flight.features.keys().count() as u32;
        let entity_position_count = flight
            .entities
            .values()
            .map(|d| {
                d.position_data
                    .as_ref()
                    .expect("No position data")
                    .position_updates
                    .len() as u32
            })
            .fold(0u32, |acc, len| acc + len);
        // Assuming ONE position per feature. Change if we support multiple.
        let position_count = entity_position_count + flight.features.len() as u32;

        let entity_event_count = flight
            .entities
            .values()
            .map(|d| d.events.len() as u32)
            .fold(0u32, |acc, len| acc + len);

        let feature_event_count = flight
            .features
            .values()
            .map(|d| d.events.len() as u32)
            .fold(0u32, |acc, len| acc + len);

        let offsets = Self {
            entity_count,
            feature_count,
            position_count,
            entity_event_count,
            feature_event_count,
        };
        offsets
    }

    fn write<W: Write>(&self, flight: &Flight, w: &mut W) -> Result<()> {
        w.write_all(b"EPAT")?;

        debug!("File size: TODO");
        write_u32(!0, w)?;

        debug!("Entity count: {}", self.entity_count);
        write_u32(self.entity_count, w)?;

        debug!("Feature count: {}", self.feature_count);
        write_u32(self.feature_count, w)?;

        debug!("Entity offset: 80");
        write_u32(80, w)?; // This header is 80 bytes long; entities start after.

        debug!("Feature offset: TODO");
        write_u32(0, w)?;

        debug!("Position count: {}", self.position_count);
        write_u32(self.position_count, w)?;

        debug!("Position offset: TODO");
        write_u32(0, w)?;

        debug!("Entity event offset: TODO");
        write_u32(0, w)?;

        debug!("General event offset: TODO");
        write_u32(0, w)?;

        debug!("General event trailer offset: TODO");
        write_u32(0, w)?;

        debug!("Text event offset: TODO");
        write_u32(0, w)?;

        debug!("Feature event offset: TODO");
        write_u32(0, w)?;

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

// pub fn write_header<W: Write>(offsets:
