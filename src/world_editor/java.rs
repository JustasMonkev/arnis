//! Java Edition Anvil format world saving.
//!
//! This module handles saving worlds in the Java Edition Anvil (.mca) format.

use super::common::{Chunk, ChunkToModify, Section};
use super::WorldEditor;
use crate::block_definitions::GRASS_BLOCK;
use crate::progress::emit_gui_progress_update;
use crate::world_utils::JAVA_DATA_VERSION;
use colored::Colorize;
use fastanvil::Region;
use fastnbt::Value;
use fnv::FnvHashMap;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// Cached base chunk sections (grass at Y=-62)
/// Computed once on first use and reused for all empty chunks
static BASE_CHUNK_SECTIONS: OnceLock<Vec<Section>> = OnceLock::new();
const DEFAULT_BIOME: &str = "minecraft:plains";
const DEFAULT_SECTION_Y: i8 = -4;

/// Get or create the cached base chunk sections
fn get_base_chunk_sections() -> &'static [Section] {
    BASE_CHUNK_SECTIONS.get_or_init(|| {
        let mut chunk = ChunkToModify::default();
        for x in 0..16 {
            for z in 0..16 {
                chunk.set_block(x, -62, z, GRASS_BLOCK);
            }
        }
        chunk.sections().collect()
    })
}

#[cfg(feature = "gui")]
use crate::telemetry::{send_log, LogLevel};

impl<'a> WorldEditor<'a> {
    /// Creates a region file for the given region coordinates.
    pub(super) fn create_region(
        &self,
        region_x: i32,
        region_z: i32,
    ) -> Result<Region<File>, Box<dyn std::error::Error + Send + Sync>> {
        let region_dir = self.world_dir.join("region");
        let out_path = region_dir.join(format!("r.{}.{}.mca", region_x, region_z));

        // Ensure region directory exists before creating region files
        std::fs::create_dir_all(&region_dir)?;

        const REGION_TEMPLATE: &[u8] = include_bytes!("../../assets/minecraft/region.template");

        let mut region_file: File = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&out_path)?;

        region_file.write_all(REGION_TEMPLATE)?;

        Ok(Region::from_stream(region_file)?)
    }

    /// Helper function to create a base chunk with grass blocks at Y -62
    /// Uses cached sections for efficiency - only serialization happens per chunk
    pub(super) fn create_base_chunk(
        abs_chunk_x: i32,
        abs_chunk_z: i32,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        // Use cached sections (computed once on first call)
        let sections = get_base_chunk_sections();

        // Prepare chunk data with cloned sections
        let chunk_data = Chunk {
            sections: sections.to_vec(),
            x_pos: abs_chunk_x,
            z_pos: abs_chunk_z,
            is_light_on: 0,
            other: FnvHashMap::default(),
        };

        // Create the Level wrapper
        let level_data = create_chunk_nbt(&chunk_data);

        // Serialize the chunk with Level wrapper
        let mut ser_buffer = Vec::with_capacity(8192);
        fastnbt::to_writer(&mut ser_buffer, &level_data)?;

        Ok(ser_buffer)
    }

    /// Saves the world in Java Edition Anvil format.
    ///
    /// Uses parallel processing with rayon for fast region saving.
    /// Returns an error if any region fails to save (e.g. disk full).
    pub(super) fn save_java(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        println!("{} Saving world...", "[7/7]".bold());
        emit_gui_progress_update(90.0, "Saving world...");

        // Save metadata with error handling
        if let Err(e) = self.save_metadata() {
            eprintln!("Failed to save world metadata: {}", e);
            #[cfg(feature = "gui")]
            send_log(LogLevel::Warning, "Failed to save world metadata.");
            // Continue with world saving even if metadata fails
        }

        let total_regions = self.world.regions.len() as u64;
        let save_pb = ProgressBar::new(total_regions);
        save_pb.set_style(
            ProgressStyle::default_bar()
                .template(
                    "{spinner:.green} [{elapsed_precise}] [{bar:45}] {pos}/{len} regions ({eta})",
                )
                .unwrap()
                .progress_chars("█▓░"),
        );

        let regions_processed = AtomicU64::new(0);
        // AtomicBool for a lock-free fast-path stop check; the Mutex only stores the error value.
        let should_stop = std::sync::atomic::AtomicBool::new(false);
        let first_error: Mutex<Option<Box<dyn std::error::Error + Send + Sync>>> = Mutex::new(None);

        self.world
            .regions
            .par_iter()
            .for_each(|((region_x, region_z), region_to_modify)| {
                // Fast-path: bail out without locking once an error has been recorded.
                if should_stop.load(Ordering::Acquire) {
                    return;
                }

                if let Err(e) = self.save_single_region(*region_x, *region_z, region_to_modify) {
                    let mut guard = first_error.lock().unwrap_or_else(|p| p.into_inner());
                    if guard.is_none() {
                        *guard = Some(e);
                    }
                    // Signal other workers to stop without re-acquiring the mutex.
                    should_stop.store(true, Ordering::Release);
                    return;
                }

                // Update progress
                let regions_done = regions_processed.fetch_add(1, Ordering::SeqCst) + 1;

                // Update progress at regular intervals (every ~10% or at least every 10 regions)
                let update_interval = (total_regions / 10).max(1);
                if regions_done.is_multiple_of(update_interval) || regions_done == total_regions {
                    let progress = 90.0 + (regions_done as f64 / total_regions as f64) * 9.0;
                    emit_gui_progress_update(progress, "Saving world...");
                }

                save_pb.inc(1);
            });

        save_pb.finish();

        if let Some(e) = first_error.lock().unwrap_or_else(|p| p.into_inner()).take() {
            return Err(e);
        }

        Ok(())
    }

    /// Saves a single region to disk.
    ///
    /// Optimized for new world creation, writes chunks directly without reading existing data.
    /// This assumes we're creating a fresh world, not modifying an existing one.
    fn save_single_region(
        &self,
        region_x: i32,
        region_z: i32,
        region_to_modify: &super::common::RegionToModify,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut region = self.create_region(region_x, region_z)?;
        let mut ser_buffer = Vec::with_capacity(8192);

        // First pass: write all chunks that have content
        for (&(chunk_x, chunk_z), chunk_to_modify) in &region_to_modify.chunks {
            if !chunk_to_modify.sections.is_empty() || !chunk_to_modify.other.is_empty() {
                // Create chunk directly, we're writing to a fresh region file
                // so there's no existing data to preserve
                let chunk = Chunk {
                    sections: chunk_to_modify.sections().collect(),
                    x_pos: chunk_x + (region_x * 32),
                    z_pos: chunk_z + (region_z * 32),
                    is_light_on: 0,
                    other: chunk_to_modify.other.clone(),
                };

                // Create Level wrapper and save
                let level_data = create_chunk_nbt(&chunk);
                ser_buffer.clear();
                fastnbt::to_writer(&mut ser_buffer, &level_data)?;
                region.write_chunk(chunk_x as usize, chunk_z as usize, &ser_buffer)?;
            }
        }

        // Second pass: ensure all chunks exist (fill with base layer if not)
        for chunk_x in 0..32 {
            for chunk_z in 0..32 {
                let abs_chunk_x = chunk_x + (region_x * 32);
                let abs_chunk_z = chunk_z + (region_z * 32);

                // Check if chunk exists in our modifications
                let chunk_exists = region_to_modify.chunks.contains_key(&(chunk_x, chunk_z));

                // If chunk doesn't exist, create it with base layer
                if !chunk_exists {
                    let ser_buffer = Self::create_base_chunk(abs_chunk_x, abs_chunk_z)?;
                    region.write_chunk(chunk_x as usize, chunk_z as usize, &ser_buffer)?;
                }
            }
        }

        Ok(())
    }
}

/// Helper function to get entity coordinates
/// Note: Currently unused since we write directly without merging, but kept for potential future use
#[inline]
#[allow(dead_code)]
fn get_entity_coords(entity: &HashMap<String, Value>) -> Option<(i32, i32, i32)> {
    if let Some(Value::List(pos)) = entity.get("Pos") {
        if pos.len() == 3 {
            if let (Some(x), Some(y), Some(z)) = (
                value_to_i32(&pos[0]),
                value_to_i32(&pos[1]),
                value_to_i32(&pos[2]),
            ) {
                return Some((x, y, z));
            }
        }
    }

    let (Some(x), Some(y), Some(z)) = (
        entity.get("x").and_then(value_to_i32),
        entity.get("y").and_then(value_to_i32),
        entity.get("z").and_then(value_to_i32),
    ) else {
        return None;
    };

    Some((x, y, z))
}

fn create_section_nbt(section: &Section) -> Value {
    let mut block_states = HashMap::from([(
        "palette".to_string(),
        Value::List(
            section
                .block_states
                .palette
                .iter()
                .map(|item| {
                    let mut palette_item =
                        HashMap::from([("Name".to_string(), Value::String(item.name.clone()))]);
                    if let Some(props) = &item.properties {
                        palette_item.insert("Properties".to_string(), props.clone());
                    }
                    Value::Compound(palette_item)
                })
                .collect(),
        ),
    )]);

    if let Some(data) = &section.block_states.data {
        if !data.is_empty() {
            block_states.insert("data".to_string(), Value::LongArray(data.to_owned()));
        }
    }

    let mut section_map = HashMap::from([
        ("Y".to_string(), Value::Byte(section.y)),
        ("block_states".to_string(), Value::Compound(block_states)),
        (
            "biomes".to_string(),
            Value::Compound(HashMap::from([(
                "palette".to_string(),
                Value::List(vec![Value::String(DEFAULT_BIOME.to_string())]),
            )])),
        ),
    ]);

    for (key, value) in &section.other {
        section_map
            .entry(key.clone())
            .or_insert_with(|| value.clone());
    }

    Value::Compound(section_map)
}

/// Creates a modern top-level chunk compound for Java Edition
#[inline]
fn create_chunk_nbt(chunk: &Chunk) -> HashMap<String, Value> {
    let min_section_y = chunk
        .sections
        .iter()
        .map(|section| section.y)
        .min()
        .unwrap_or(DEFAULT_SECTION_Y);

    let mut chunk_map = HashMap::from([
        ("DataVersion".to_string(), Value::Int(JAVA_DATA_VERSION)),
        (
            "Status".to_string(),
            Value::String("minecraft:full".to_string()),
        ),
        ("xPos".to_string(), Value::Int(chunk.x_pos)),
        ("zPos".to_string(), Value::Int(chunk.z_pos)),
        ("yPos".to_string(), Value::Int(i32::from(min_section_y))),
        ("LastUpdate".to_string(), Value::Long(0)),
        ("InhabitedTime".to_string(), Value::Long(0)),
        (
            "Heightmaps".to_string(),
            Value::Compound(HashMap::default()),
        ),
        (
            "sections".to_string(),
            Value::List(chunk.sections.iter().map(create_section_nbt).collect()),
        ),
        (
            "block_entities".to_string(),
            chunk
                .other
                .get("block_entities")
                .cloned()
                .unwrap_or_else(|| Value::List(Vec::new())),
        ),
        (
            "structures".to_string(),
            Value::Compound(HashMap::from([
                ("starts".to_string(), Value::Compound(HashMap::default())),
                (
                    "References".to_string(),
                    Value::Compound(HashMap::default()),
                ),
            ])),
        ),
    ]);

    for (key, value) in &chunk.other {
        if key != "block_entities" {
            chunk_map.insert(key.clone(), value.clone());
        }
    }

    chunk_map
}

/// Merge compound lists (entities, block_entities) from chunk_to_modify into chunk
/// Note: Currently unused since we write directly without merging, but kept for potential future use
#[allow(dead_code)]
fn merge_compound_list(chunk: &mut Chunk, chunk_to_modify: &ChunkToModify, key: &str) {
    if let Some(existing_entities) = chunk.other.get_mut(key) {
        if let Some(new_entities) = chunk_to_modify.other.get(key) {
            if let (Value::List(existing), Value::List(new)) = (existing_entities, new_entities) {
                existing.retain(|e| {
                    if let Value::Compound(map) = e {
                        if let Some((x, y, z)) = get_entity_coords(map) {
                            return !new.iter().any(|new_e| {
                                if let Value::Compound(new_map) = new_e {
                                    get_entity_coords(new_map) == Some((x, y, z))
                                } else {
                                    false
                                }
                            });
                        }
                    }
                    true
                });
                existing.extend(new.clone());
            }
        }
    } else if let Some(new_entities) = chunk_to_modify.other.get(key) {
        chunk.other.insert(key.to_string(), new_entities.clone());
    }
}

/// Convert NBT Value to i32
/// Note: Currently unused since we write directly without merging, but kept for potential future use
#[allow(dead_code)]
fn value_to_i32(value: &Value) -> Option<i32> {
    match value {
        Value::Byte(v) => Some(i32::from(*v)),
        Value::Short(v) => Some(i32::from(*v)),
        Value::Int(v) => Some(*v),
        Value::Long(v) => i32::try_from(*v).ok(),
        Value::Float(v) => Some(*v as i32),
        Value::Double(v) => Some(*v as i32),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world_editor::common::{Blockstates, PaletteItem, Section};

    #[test]
    fn create_chunk_nbt_uses_modern_root_fields() {
        let chunk = Chunk {
            sections: vec![Section {
                block_states: Blockstates {
                    palette: vec![PaletteItem {
                        name: "minecraft:stone".to_string(),
                        properties: None,
                    }],
                    data: None,
                    other: FnvHashMap::default(),
                },
                y: 0,
                other: FnvHashMap::default(),
            }],
            x_pos: 12,
            z_pos: 34,
            is_light_on: 0,
            other: FnvHashMap::default(),
        };

        let nbt = create_chunk_nbt(&chunk);

        assert_eq!(nbt.get("DataVersion"), Some(&Value::Int(JAVA_DATA_VERSION)));
        assert_eq!(
            nbt.get("Status"),
            Some(&Value::String("minecraft:full".to_string()))
        );
        assert!(!nbt.contains_key("Level"));

        let sections = match nbt.get("sections") {
            Some(Value::List(sections)) => sections,
            other => panic!("expected sections list, got {other:?}"),
        };

        let section = match &sections[0] {
            Value::Compound(section) => section,
            other => panic!("expected section compound, got {other:?}"),
        };

        assert!(section.contains_key("biomes"));
    }
}
