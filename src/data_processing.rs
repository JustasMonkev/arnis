use crate::args::Args;
use crate::block_definitions::{BEDROCK, DIRT, GRASS_BLOCK, SMOOTH_STONE, STONE};
use crate::coordinate_system::cartesian::XZBBox;
use crate::coordinate_system::geographic::LLBBox;
use crate::element_processing::*;
use crate::floodfill_cache::{BuildingFootprintBitmap, FloodFillCache};
use crate::ground::Ground;
use crate::map_renderer;
use crate::osm_parser::{get_priority, ProcessedElement, ProcessedMemberRole};
use crate::progress::{emit_gui_progress_update, emit_map_preview_ready, emit_open_mcworld_file};
#[cfg(feature = "gui")]
use crate::telemetry::{send_log, LogLevel};
use crate::urban_ground;
use crate::world_editor::{WorldEditor, WorldFormat, WorldToModify};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub const MIN_Y: i32 = -64;

/// Generation options that can be passed separately from CLI Args
#[derive(Clone)]
pub struct GenerationOptions {
    pub path: PathBuf,
    pub format: WorldFormat,
    pub level_name: Option<String>,
    pub spawn_point: Option<(i32, i32)>,
}

const PARALLEL_PRIORITY_THRESHOLD: usize = 1;
const SEQUENTIAL_PRIORITY_THRESHOLD: usize = 6;

fn should_parallelize_priority(priority: usize) -> bool {
    (PARALLEL_PRIORITY_THRESHOLD..SEQUENTIAL_PRIORITY_THRESHOLD).contains(&priority)
}

#[allow(clippy::too_many_arguments)]
fn process_one_element(
    editor: &mut WorldEditor,
    element: &ProcessedElement,
    args: &Args,
    xzbbox: &XZBBox,
    highway_connectivity: &highways::HighwayConnectivityMap,
    flood_fill_cache: &FloodFillCache,
    building_footprints: &BuildingFootprintBitmap,
    suppressed_building_outlines: &HashSet<u64>,
) {
    match element {
        ProcessedElement::Way(way) => {
            if way.tags.contains_key("building") || way.tags.contains_key("building:part") {
                if !suppressed_building_outlines.contains(&way.id) {
                    buildings::generate_buildings(editor, way, args, None, None, flood_fill_cache);
                }
            } else if way.tags.contains_key("highway") {
                highways::generate_highways(
                    editor,
                    element,
                    args,
                    highway_connectivity,
                    flood_fill_cache,
                );
            } else if way.tags.contains_key("landuse") {
                landuse::generate_landuse(editor, way, args, flood_fill_cache, building_footprints);
            } else if way.tags.contains_key("natural") {
                natural::generate_natural(
                    editor,
                    element,
                    args,
                    flood_fill_cache,
                    building_footprints,
                );
            } else if way.tags.contains_key("amenity") {
                amenities::generate_amenities(editor, element, args, flood_fill_cache);
            } else if way.tags.contains_key("leisure") {
                leisure::generate_leisure(editor, way, args, flood_fill_cache, building_footprints);
            } else if way.tags.contains_key("barrier") {
                barriers::generate_barriers(editor, element);
            } else if let Some(val) = way.tags.get("waterway") {
                if val == "dock" {
                    water_areas::generate_water_area_from_way(editor, way, xzbbox);
                } else {
                    waterways::generate_waterways(editor, way);
                }
            } else if way.tags.contains_key("railway") {
                railways::generate_railways(editor, way);
            } else if way.tags.contains_key("roller_coaster") {
                railways::generate_roller_coaster(editor, way);
            } else if way.tags.contains_key("aeroway") || way.tags.contains_key("area:aeroway") {
                highways::generate_aeroway(editor, way, args);
            } else if way.tags.get("service") == Some(&"siding".to_string()) {
                highways::generate_siding(editor, way);
            } else if way.tags.get("tomb") == Some(&"pyramid".to_string()) {
                historic::generate_pyramid(editor, way, args, flood_fill_cache);
            } else if way.tags.contains_key("man_made") {
                man_made::generate_man_made(editor, element, args);
            } else if way.tags.contains_key("power") {
                power::generate_power(editor, element);
            } else if way.tags.contains_key("place") {
                landuse::generate_place(editor, way, args, flood_fill_cache);
            }
        }
        ProcessedElement::Node(node) => {
            if node.tags.contains_key("door") || node.tags.contains_key("entrance") {
                doors::generate_doors(editor, node);
            } else if node.tags.contains_key("natural")
                && node.tags.get("natural") == Some(&"tree".to_string())
            {
                natural::generate_natural(
                    editor,
                    element,
                    args,
                    flood_fill_cache,
                    building_footprints,
                );
            } else if node.tags.contains_key("amenity") {
                amenities::generate_amenities(editor, element, args, flood_fill_cache);
            } else if node.tags.contains_key("barrier") {
                barriers::generate_barrier_nodes(editor, node);
            } else if node.tags.contains_key("highway") {
                highways::generate_highways(
                    editor,
                    element,
                    args,
                    highway_connectivity,
                    flood_fill_cache,
                );
            } else if node.tags.contains_key("tourism") {
                tourisms::generate_tourisms(editor, node);
            } else if node.tags.contains_key("man_made") {
                man_made::generate_man_made_nodes(editor, node);
            } else if node.tags.contains_key("power") {
                power::generate_power_nodes(editor, node);
            } else if node.tags.contains_key("historic") {
                historic::generate_historic(editor, node);
            } else if node.tags.contains_key("emergency") {
                emergency::generate_emergency(editor, node);
            } else if node.tags.contains_key("advertising") {
                advertising::generate_advertising(editor, node);
            }
        }
        ProcessedElement::Relation(rel) => {
            let is_building_relation = rel.tags.contains_key("building")
                || rel.tags.contains_key("building:part")
                || rel.tags.get("type").map(|t| t.as_str()) == Some("building");
            if is_building_relation {
                buildings::generate_building_from_relation(
                    editor,
                    rel,
                    args,
                    flood_fill_cache,
                    xzbbox,
                );
            } else if rel.tags.contains_key("water")
                || rel
                    .tags
                    .get("natural")
                    .map(|val| val == "water" || val == "bay")
                    .unwrap_or(false)
            {
                water_areas::generate_water_areas_from_relation(editor, rel, xzbbox);
            } else if rel.tags.contains_key("natural") {
                natural::generate_natural_from_relation(
                    editor,
                    rel,
                    args,
                    flood_fill_cache,
                    building_footprints,
                );
            } else if rel.tags.contains_key("landuse") {
                landuse::generate_landuse_from_relation(
                    editor,
                    rel,
                    args,
                    flood_fill_cache,
                    building_footprints,
                );
            } else if rel.tags.get("leisure") == Some(&"park".to_string()) {
                leisure::generate_leisure_from_relation(
                    editor,
                    rel,
                    args,
                    flood_fill_cache,
                    building_footprints,
                );
            } else if rel.tags.contains_key("man_made") {
                man_made::generate_man_made(editor, element, args);
            }
        }
    }
}

/// Generate world with explicit format options (used by GUI for Bedrock support)
pub fn generate_world_with_options(
    elements: Vec<ProcessedElement>,
    xzbbox: XZBBox,
    llbbox: LLBBox,
    ground: Ground,
    args: &Args,
    options: GenerationOptions,
) -> Result<PathBuf, String> {
    let output_path = options.path.clone();
    let world_format = options.format;

    // Create editor with appropriate format
    let mut editor: WorldEditor = WorldEditor::new_with_format_and_name(
        options.path,
        &xzbbox,
        llbbox,
        options.format,
        options.level_name.clone(),
        options.spawn_point,
    );
    let ground = Arc::new(ground);

    println!("{} Processing data...", "[4/7]".bold());

    // Build highway connectivity map once before processing
    let highway_connectivity = highways::build_highway_connectivity_map(&elements);

    // Set ground reference in the editor to enable elevation-aware block placement
    editor.set_ground(Arc::clone(&ground));

    println!("{} Processing terrain...", "[5/7]".bold());
    emit_gui_progress_update(25.0, "Processing terrain...");

    // Pre-compute all flood fills in parallel for better CPU utilization.
    // This stays immutable during element processing so parallel priority stages
    // can share it safely.
    let flood_fill_cache = FloodFillCache::precompute(&elements, args.timeout.as_ref());

    // Collect building footprints to prevent trees from spawning inside buildings
    // Uses a memory-efficient bitmap (~1 bit per coordinate) instead of a HashSet (~24 bytes per coordinate)
    let building_footprints = flood_fill_cache.collect_building_footprints(&elements, &xzbbox);

    // Collect building centroids for urban ground generation (only if enabled)
    // This must be done before the processing loop clears the flood fill cache
    let building_centroids = if args.city_boundaries {
        flood_fill_cache.collect_building_centroids(&elements)
    } else {
        Vec::new()
    };

    // Process all elements (no longer need to partition boundaries)
    let elements_count: usize = elements.len();
    let process_pb: ProgressBar = ProgressBar::new(elements_count as u64);
    process_pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:45.white/black}] {pos}/{len} elements ({eta}) {msg}")
        .unwrap()
        .progress_chars("█▓░"));

    let processed_elements = AtomicU64::new(0);
    let progress_lock = Mutex::new((25.0_f64, 25.0_f64));

    // Pre-scan: detect building relation outlines that should be suppressed.
    // Only applies to type=building relations (NOT type=multipolygon).
    // When a type=building relation has "part" members, the outline way should not
    // render as a standalone building, the individual parts render instead.
    let suppressed_building_outlines: HashSet<u64> = {
        let mut outlines = HashSet::new();
        for element in &elements {
            if let ProcessedElement::Relation(rel) = element {
                let is_building_type = rel.tags.get("type").map(|t| t.as_str()) == Some("building");
                if is_building_type {
                    let has_parts = rel
                        .members
                        .iter()
                        .any(|m| m.role == ProcessedMemberRole::Part);
                    if has_parts {
                        for member in &rel.members {
                            if member.role == ProcessedMemberRole::Outer {
                                outlines.insert(member.way.id);
                            }
                        }
                    }
                }
            }
        }
        outlines
    };

    let thread_count = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .max(1);

    let mut bucket_start = 0usize;
    while bucket_start < elements.len() {
        let priority = get_priority(&elements[bucket_start]);
        let mut bucket_end = bucket_start + 1;
        while bucket_end < elements.len() && get_priority(&elements[bucket_end]) == priority {
            bucket_end += 1;
        }

        let bucket = &elements[bucket_start..bucket_end];

        if should_parallelize_priority(priority) && bucket.len() > 1 {
            let batch_count = (thread_count * 2).min(bucket.len()).max(1);
            let batch_size = bucket.len().div_ceil(batch_count);
            let batched_world = Mutex::new(WorldToModify::default());

            bucket.par_chunks(batch_size).for_each(|batch| {
                let mut local_editor = WorldEditor::new_with_format_and_name(
                    PathBuf::new(),
                    &xzbbox,
                    llbbox,
                    world_format,
                    None,
                    None,
                );
                local_editor.set_ground(Arc::clone(&ground));

                for element in batch {
                    process_one_element(
                        &mut local_editor,
                        element,
                        args,
                        &xzbbox,
                        &highway_connectivity,
                        &flood_fill_cache,
                        &building_footprints,
                        &suppressed_building_outlines,
                    );
                }

                let mut merged_world = batched_world
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                merged_world.merge_from(local_editor.into_modifications());

                let done = processed_elements.fetch_add(batch.len() as u64, Ordering::Relaxed)
                    + batch.len() as u64;
                process_pb.inc(batch.len() as u64);
                let mut progress = progress_lock
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                progress.0 = 25.0 + (done as f64 / elements_count as f64) * 45.0;
                if (progress.0 - progress.1).abs() > 0.25 {
                    emit_gui_progress_update(progress.0, "");
                    progress.1 = progress.0;
                }
            });

            let batched_world = batched_world
                .into_inner()
                .unwrap_or_else(|poison| poison.into_inner());
            editor.merge_modifications(batched_world);
        } else {
            for element in bucket {
                if args.debug {
                    process_pb.set_message(format!(
                        "(Element ID: {} / Type: {})",
                        element.id(),
                        element.kind()
                    ));
                } else {
                    process_pb.set_message("");
                }

                process_one_element(
                    &mut editor,
                    element,
                    args,
                    &xzbbox,
                    &highway_connectivity,
                    &flood_fill_cache,
                    &building_footprints,
                    &suppressed_building_outlines,
                );

                let done = processed_elements.fetch_add(1, Ordering::Relaxed) + 1;
                process_pb.inc(1);
                let mut progress = progress_lock
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                progress.0 = 25.0 + (done as f64 / elements_count as f64) * 45.0;
                if (progress.0 - progress.1).abs() > 0.25 {
                    emit_gui_progress_update(progress.0, "");
                    progress.1 = progress.0;
                }
            }
        }

        bucket_start = bucket_end;
    }

    process_pb.finish();

    // Compute urban ground lookup (if enabled)
    // Uses a compact cell-based representation instead of storing all coordinates.
    // Memory usage: ~270 KB vs ~560 MB for coordinate-based approach.
    let urban_lookup = if args.city_boundaries && !building_centroids.is_empty() {
        urban_ground::compute_urban_ground_lookup(building_centroids, &xzbbox)
    } else {
        urban_ground::UrbanGroundLookup::empty()
    };
    let has_urban_ground = !urban_lookup.is_empty();

    // Drop remaining caches
    drop(highway_connectivity);
    drop(flood_fill_cache);

    println!("{} Generating ground...", "[6/7]".bold());
    emit_gui_progress_update(70.0, "Generating ground...");

    // Check if terrain elevation is enabled; when disabled, we can skip ground level lookups entirely
    let terrain_enabled = ground.elevation_enabled;

    // Process ground generation chunk-by-chunk for better cache locality.
    // This keeps the same region/chunk HashMap entries hot in CPU cache,
    // rather than jumping between regions on every Z iteration.
    let min_chunk_x = xzbbox.min_x() >> 4;
    let max_chunk_x = xzbbox.max_x() >> 4;
    let min_chunk_z = xzbbox.min_z() >> 4;
    let max_chunk_z = xzbbox.max_z() >> 4;

    let total_chunks: u64 =
        ((max_chunk_x - min_chunk_x + 1) as u64) * ((max_chunk_z - min_chunk_z + 1) as u64);
    let progress_update_interval = (total_chunks / 80).max(1);
    let ground_pb: ProgressBar = ProgressBar::new(total_chunks);
    ground_pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:45}] {pos}/{len} chunks ({eta})")
            .unwrap()
            .progress_chars("█▓░"),
    );

    let chunk_coords: Vec<(i32, i32)> = (min_chunk_x..=max_chunk_x)
        .flat_map(|chunk_x| (min_chunk_z..=max_chunk_z).map(move |chunk_z| (chunk_x, chunk_z)))
        .collect();
    let editor_view = &editor;
    let ground_world = Mutex::new(WorldToModify::default());
    let chunks_done = AtomicU64::new(0);

    chunk_coords.into_par_iter().for_each(|(chunk_x, chunk_z)| {
        let mut local_world = WorldToModify::default();

        // Calculate the block range for this chunk, clamped to bbox
        let chunk_min_x = (chunk_x << 4).max(xzbbox.min_x());
        let chunk_max_x = ((chunk_x << 4) + 15).min(xzbbox.max_x());
        let chunk_min_z = (chunk_z << 4).max(xzbbox.min_z());
        let chunk_max_z = ((chunk_z << 4) + 15).min(xzbbox.max_z());

        for x in chunk_min_x..=chunk_max_x {
            for z in chunk_min_z..=chunk_max_z {
                let ground_y = if terrain_enabled {
                    editor_view.get_ground_level(x, z)
                } else {
                    args.ground_level
                };

                let is_urban = has_urban_ground && urban_lookup.is_urban(x, z);

                if !editor_view.check_for_block_absolute(x, ground_y, z, Some(&[STONE]), None) {
                    let surface_block = if is_urban { SMOOTH_STONE } else { GRASS_BLOCK };

                    if !editor_view.block_at_absolute(x, ground_y, z) {
                        local_world.set_block(x, ground_y, z, surface_block);
                    }
                    if !editor_view.block_at_absolute(x, ground_y - 1, z) {
                        local_world.set_block(x, ground_y - 1, z, DIRT);
                    }
                    if !editor_view.block_at_absolute(x, ground_y - 2, z) {
                        local_world.set_block(x, ground_y - 2, z, DIRT);
                    }
                }

                if args.fillground {
                    for y in (MIN_Y + 1)..=(ground_y - 3) {
                        if !editor_view.block_at_absolute(x, y, z) {
                            local_world.set_block(x, y, z, STONE);
                        }
                    }
                }

                if !editor_view.check_for_block_absolute(x, MIN_Y, z, Some(&[BEDROCK]), None) {
                    local_world.set_block(x, MIN_Y, z, BEDROCK);
                }
            }
        }

        if !local_world.regions.is_empty() {
            let mut merged_world = ground_world
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            merged_world.merge_from(local_world);
        }

        let done = chunks_done.fetch_add(1, Ordering::Relaxed) + 1;
        ground_pb.inc(1);
        if done.is_multiple_of(progress_update_interval) || done == total_chunks {
            let progress = 70.0 + (done as f64 / total_chunks as f64) * 20.0;
            emit_gui_progress_update(progress, "");
        }
    });

    // Set sign for player orientation
    /*editor.set_sign(
        "↑".to_string(),
        "Generated World".to_string(),
        "This direction".to_string(),
        "".to_string(),
        9,
        -61,
        9,
        6,
    );*/

    ground_pb.finish();
    let ground_world = ground_world
        .into_inner()
        .unwrap_or_else(|poison| poison.into_inner());
    editor.merge_modifications(ground_world);

    // Save world
    if let Err(e) = editor.save() {
        return Err(e.to_string());
    }

    emit_gui_progress_update(99.0, "Finalizing world...");

    // Update player spawn Y coordinate based on terrain height after generation
    #[cfg(feature = "gui")]
    if world_format == WorldFormat::JavaAnvil {
        use crate::gui::update_player_spawn_y_after_generation;
        // Reconstruct bbox string to match the format that GUI originally provided.
        // This ensures LLBBox::from_str() can parse it correctly.
        let bbox_string = format!(
            "{},{},{},{}",
            args.bbox.min().lat(),
            args.bbox.min().lng(),
            args.bbox.max().lat(),
            args.bbox.max().lng()
        );

        // Always update spawn Y since we now always set a spawn point (user-selected or default)
        if let Some(ref world_path) = args.path {
            if let Err(e) = update_player_spawn_y_after_generation(
                world_path,
                bbox_string,
                args.scale,
                ground.as_ref(),
            ) {
                let warning_msg = format!("Failed to update spawn point Y coordinate: {}", e);
                eprintln!("Warning: {}", warning_msg);
                #[cfg(feature = "gui")]
                send_log(LogLevel::Warning, &warning_msg);
            }
        }
    }

    // For Bedrock format, emit event to open the mcworld file
    if world_format == WorldFormat::BedrockMcWorld {
        if let Some(path_str) = output_path.to_str() {
            emit_open_mcworld_file(path_str);
        }
    }

    Ok(output_path)
}

/// Information needed to generate a map preview after world generation is complete
#[derive(Clone)]
pub struct MapPreviewInfo {
    pub world_path: PathBuf,
    pub min_x: i32,
    pub max_x: i32,
    pub min_z: i32,
    pub max_z: i32,
    pub world_area: i64,
}

impl MapPreviewInfo {
    /// Create MapPreviewInfo from world bounds
    pub fn new(world_path: PathBuf, xzbbox: &XZBBox) -> Self {
        let world_width = (xzbbox.max_x() - xzbbox.min_x()) as i64;
        let world_height = (xzbbox.max_z() - xzbbox.min_z()) as i64;
        Self {
            world_path,
            min_x: xzbbox.min_x(),
            max_x: xzbbox.max_x(),
            min_z: xzbbox.min_z(),
            max_z: xzbbox.max_z(),
            world_area: world_width * world_height,
        }
    }
}

/// Maximum area for which map preview generation is allowed (to avoid memory issues)
pub const MAX_MAP_PREVIEW_AREA: i64 = 6400 * 6900;

/// Start map preview generation in a background thread.
/// This should be called AFTER the world generation is complete, the session lock is released,
/// and the GUI has been notified of 100% completion.
///
/// For Java worlds only, and only if the world area is within limits.
pub fn start_map_preview_generation(info: MapPreviewInfo) {
    if info.world_area > MAX_MAP_PREVIEW_AREA {
        return;
    }

    std::thread::spawn(move || {
        // Use catch_unwind to prevent any panic from affecting the application
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            map_renderer::render_world_map(
                &info.world_path,
                info.min_x,
                info.max_x,
                info.min_z,
                info.max_z,
            )
        }));

        match result {
            Ok(Ok(_path)) => {
                // Notify the GUI that the map preview is ready
                emit_map_preview_ready();
            }
            Ok(Err(e)) => {
                eprintln!("Warning: Failed to generate map preview: {}", e);
            }
            Err(_) => {
                eprintln!("Warning: Map preview generation panicked unexpectedly");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::should_parallelize_priority;

    #[test]
    fn priority_parallelization_matches_staged_plan() {
        assert!(!should_parallelize_priority(0));
        assert!(should_parallelize_priority(1));
        assert!(should_parallelize_priority(2));
        assert!(should_parallelize_priority(3));
        assert!(should_parallelize_priority(4));
        assert!(should_parallelize_priority(5));
        assert!(!should_parallelize_priority(6));
        assert!(!should_parallelize_priority(7));
    }
}
