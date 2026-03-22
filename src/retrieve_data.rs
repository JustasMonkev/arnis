use crate::coordinate_system::geographic::LLBBox;
use crate::osm_parser::OsmData;
use crate::progress::{emit_gui_error, emit_gui_progress_update, is_running_with_gui};
#[cfg(feature = "gui")]
use crate::telemetry::{send_log, LogLevel};
use colored::Colorize;
use rand::prelude::SliceRandom;
use reqwest::blocking::Client;
use reqwest::blocking::ClientBuilder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::File;
use std::io::{self, BufReader, Cursor, Write};
use std::process::Command;
use std::time::Duration;

const OVERPASS_TIMEOUT_SECS: u64 = 360;
const INITIAL_TILE_MAX_SPAN_DEGREES: f64 = 0.03;
const MAX_TILES_PER_AXIS: usize = 8;
const MAX_OVERPASS_SPLIT_DEPTH: u8 = 3;

#[derive(Debug, Deserialize, Serialize)]
struct RawOverpassResponse {
    #[serde(default)]
    elements: Vec<Value>,
    #[serde(default)]
    remark: Option<String>,
}

/// Function to download data using reqwest
fn download_with_reqwest(url: &str, query: &str) -> Result<String, Box<dyn std::error::Error>> {
    let client: Client = ClientBuilder::new()
        .timeout(Duration::from_secs(OVERPASS_TIMEOUT_SECS))
        .build()?;

    let response: Result<reqwest::blocking::Response, reqwest::Error> =
        client.get(url).query(&[("data", query)]).send();

    match response {
        Ok(resp) => {
            emit_gui_progress_update(3.0, "Downloading data...");
            if resp.status().is_success() {
                let text = resp.text()?;
                if text.is_empty() {
                    return Err("Error! Received invalid from server".into());
                }
                Ok(text)
            } else {
                Err(format!("Error! Received response code: {}", resp.status()).into())
            }
        }
        Err(e) => {
            if e.is_timeout() {
                let msg = "Request timed out. Try selecting a smaller area.";
                eprintln!("{}", format!("Error! {msg}").red().bold());
                Err(msg.into())
            } else if e.is_connect() {
                let msg = "No internet connection.";
                eprintln!("{}", format!("Error! {msg}").red().bold());
                Err(msg.into())
            } else {
                #[cfg(feature = "gui")]
                send_log(
                    LogLevel::Error,
                    &format!("Request error in download_with_reqwest: {e}"),
                );
                eprintln!("{}", format!("Error! {e:.52}").red().bold());
                Err(format!("{e:.52}").into())
            }
        }
    }
}

/// Function to download data using `curl`
fn download_with_curl(url: &str, query: &str) -> io::Result<String> {
    let output: std::process::Output = Command::new("curl")
        .arg("-s") // Add silent mode to suppress output
        .arg(format!("{url}?data={query}"))
        .output()?;

    if !output.status.success() {
        Err(io::Error::other("Curl command failed"))
    } else {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

/// Function to download data using `wget`
fn download_with_wget(url: &str, query: &str) -> io::Result<String> {
    let output: std::process::Output = Command::new("wget")
        .arg("-qO-") // Use `-qO-` to output the result directly to stdout
        .arg(format!("{url}?data={query}"))
        .output()?;

    if !output.status.success() {
        Err(io::Error::other("Wget command failed"))
    } else {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

pub fn fetch_data_from_file(file: &str) -> Result<OsmData, Box<dyn std::error::Error>> {
    println!("{} Loading data from file...", "[1/7]".bold());
    emit_gui_progress_update(1.0, "Loading data from file...");

    let file: File = File::open(file)?;
    let reader: BufReader<File> = BufReader::new(file);
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    let data: OsmData = OsmData::deserialize(&mut deserializer)?;
    Ok(data)
}

fn build_overpass_query(bbox: LLBBox) -> String {
    format!(
        r#"[out:json][timeout:{}][bbox:{},{},{},{}];
    (
        nwr["building"];
        nwr["building:part"];
        nwr["highway"];
        nwr["landuse"];
        nwr["natural"];
        nwr["leisure"];
        nwr["water"];
        nwr["waterway"];
        nwr["amenity"];
        nwr["tourism"];
        nwr["bridge"];
        nwr["railway"];
        nwr["roller_coaster"];
        nwr["barrier"];
        nwr["entrance"];
        nwr["door"];
        nwr["power"];
        nwr["historic"];
        nwr["emergency"];
        nwr["advertising"];
        nwr["man_made"];
        nwr["aeroway"];
        way["place"];
    )->.relsinbbox;
    (
        way(r.relsinbbox);
    )->.waysinbbox;
    (
        node(w.waysinbbox);
        node(w.relsinbbox);
    )->.nodesinbbox;
    .relsinbbox out body;
    .waysinbbox out body;
    .nodesinbbox out skel qt;"#,
        OVERPASS_TIMEOUT_SECS,
        bbox.min().lat(),
        bbox.min().lng(),
        bbox.max().lat(),
        bbox.max().lng(),
    )
}

fn download_overpass_query(
    url: &str,
    query: &str,
    download_method: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    match download_method {
        "requests" => download_with_reqwest(url, query),
        "curl" => download_with_curl(url, query).map_err(|e| e.into()),
        "wget" => download_with_wget(url, query).map_err(|e| e.into()),
        _ => download_with_reqwest(url, query),
    }
}

fn parse_raw_overpass_response(
    response: &str,
) -> Result<RawOverpassResponse, Box<dyn std::error::Error>> {
    let mut deserializer = serde_json::Deserializer::from_reader(Cursor::new(response.as_bytes()));
    let data: RawOverpassResponse = RawOverpassResponse::deserialize(&mut deserializer)?;
    Ok(data)
}

fn overpass_error_suggests_split(message: &str) -> bool {
    let msg = message.to_lowercase();
    msg.contains("timed out")
        || msg.contains("out of memory")
        || msg.contains("response code: 429")
        || msg.contains("response code: 502")
        || msg.contains("response code: 503")
        || msg.contains("response code: 504")
}

fn overpass_remark_suggests_split(remark: &str) -> bool {
    let msg = remark.to_lowercase();
    msg.contains("out of memory")
        || msg.contains("timed out")
        || msg.contains("timeout")
        || msg.contains("runtime error")
}

fn tile_count_for_span(span: f64) -> usize {
    ((span / INITIAL_TILE_MAX_SPAN_DEGREES).ceil() as usize).clamp(1, MAX_TILES_PER_AXIS)
}

fn initial_tiles_for_bbox(bbox: LLBBox) -> Vec<LLBBox> {
    let lat_tiles = tile_count_for_span(bbox.max().lat() - bbox.min().lat());
    let lng_tiles = tile_count_for_span(bbox.max().lng() - bbox.min().lng());
    split_bbox_into_grid(bbox, lat_tiles, lng_tiles)
}

fn split_bbox_into_grid(bbox: LLBBox, lat_tiles: usize, lng_tiles: usize) -> Vec<LLBBox> {
    let lat_tiles = lat_tiles.max(1);
    let lng_tiles = lng_tiles.max(1);
    let lat_step = (bbox.max().lat() - bbox.min().lat()) / lat_tiles as f64;
    let lng_step = (bbox.max().lng() - bbox.min().lng()) / lng_tiles as f64;

    let mut tiles = Vec::with_capacity(lat_tiles * lng_tiles);
    for lat_idx in 0..lat_tiles {
        let min_lat = bbox.min().lat() + lat_step * lat_idx as f64;
        let max_lat = if lat_idx + 1 == lat_tiles {
            bbox.max().lat()
        } else {
            bbox.min().lat() + lat_step * (lat_idx + 1) as f64
        };

        for lng_idx in 0..lng_tiles {
            let min_lng = bbox.min().lng() + lng_step * lng_idx as f64;
            let max_lng = if lng_idx + 1 == lng_tiles {
                bbox.max().lng()
            } else {
                bbox.min().lng() + lng_step * (lng_idx + 1) as f64
            };

            if let Ok(tile) = LLBBox::new(min_lat, min_lng, max_lat, max_lng) {
                tiles.push(tile);
            }
        }
    }

    tiles
}

fn element_identity(element: &Value) -> Option<(String, u64)> {
    let obj = element.as_object()?;
    let element_type = obj.get("type")?.as_str()?.to_string();
    let id = obj.get("id")?.as_u64()?;
    Some((element_type, id))
}

fn merge_raw_overpass_responses(responses: Vec<RawOverpassResponse>) -> RawOverpassResponse {
    let mut seen = std::collections::HashSet::new();
    let mut merged_elements = Vec::new();
    let mut remarks = Vec::new();

    for response in responses {
        if let Some(remark) = response.remark {
            if !remark.is_empty() {
                remarks.push(remark);
            }
        }

        for element in response.elements {
            if let Some(identity) = element_identity(&element) {
                if seen.insert(identity) {
                    merged_elements.push(element);
                }
            } else {
                merged_elements.push(element);
            }
        }
    }

    RawOverpassResponse {
        elements: merged_elements,
        remark: if remarks.is_empty() {
            None
        } else {
            Some(remarks.join(" | "))
        },
    }
}

fn fetch_single_bbox_raw(
    bbox: LLBBox,
    download_method: &str,
    api_servers: &[&str],
    fallback_api_servers: &[&str],
) -> Result<RawOverpassResponse, Box<dyn std::error::Error>> {
    let query = build_overpass_query(bbox);
    let mut all_servers: Vec<&str> = api_servers.to_vec();
    all_servers.shuffle(&mut rand::rng());
    all_servers.extend_from_slice(fallback_api_servers);

    let mut last_error: Option<Box<dyn std::error::Error>> = None;
    let mut split_error: Option<Box<dyn std::error::Error>> = None;
    for url in all_servers {
        println!("Downloading from {url} with method {download_method}...");
        match download_overpass_query(url, &query, download_method) {
            Ok(response) => return parse_raw_overpass_response(&response),
            Err(error) => {
                eprintln!("Request failed on {url}: {error}");
                if split_error.is_none() && overpass_error_suggests_split(&error.to_string()) {
                    split_error = Some(error.to_string().into());
                }
                last_error = Some(error);
            }
        }
    }

    Err(split_error
        .or(last_error)
        .unwrap_or_else(|| "No Overpass servers available.".into()))
}

fn fetch_bbox_adaptive(
    bbox: LLBBox,
    download_method: &str,
    api_servers: &[&str],
    fallback_api_servers: &[&str],
    depth: u8,
    allow_empty: bool,
) -> Result<RawOverpassResponse, Box<dyn std::error::Error>> {
    if depth == 0 {
        let initial_tiles = initial_tiles_for_bbox(bbox);
        if initial_tiles.len() > 1 {
            let mut tile_responses = Vec::with_capacity(initial_tiles.len());
            for tile in initial_tiles {
                tile_responses.push(fetch_bbox_adaptive(
                    tile,
                    download_method,
                    api_servers,
                    fallback_api_servers,
                    depth + 1,
                    true,
                )?);
            }
            return Ok(merge_raw_overpass_responses(tile_responses));
        }
    }

    match fetch_single_bbox_raw(bbox, download_method, api_servers, fallback_api_servers) {
        Ok(response) => {
            if let Some(remark) = response.remark.as_deref() {
                if overpass_remark_suggests_split(remark) && depth < MAX_OVERPASS_SPLIT_DEPTH {
                    let tiles = split_bbox_into_grid(bbox, 2, 2);
                    if tiles.len() > 1 {
                        let mut tile_responses = Vec::with_capacity(tiles.len());
                        for tile in tiles {
                            tile_responses.push(fetch_bbox_adaptive(
                                tile,
                                download_method,
                                api_servers,
                                fallback_api_servers,
                                depth + 1,
                                true,
                            )?);
                        }
                        return Ok(merge_raw_overpass_responses(tile_responses));
                    }
                }
            }

            if response.elements.is_empty() && !allow_empty {
                return Ok(response);
            }

            Ok(response)
        }
        Err(error) => {
            let message = error.to_string();
            if overpass_error_suggests_split(&message) && depth < MAX_OVERPASS_SPLIT_DEPTH {
                let tiles = split_bbox_into_grid(bbox, 2, 2);
                if tiles.len() > 1 {
                    let mut tile_responses = Vec::with_capacity(tiles.len());
                    for tile in tiles {
                        tile_responses.push(fetch_bbox_adaptive(
                            tile,
                            download_method,
                            api_servers,
                            fallback_api_servers,
                            depth + 1,
                            true,
                        )?);
                    }
                    return Ok(merge_raw_overpass_responses(tile_responses));
                }
            }

            Err(error)
        }
    }
}

/// Main function to fetch data
pub fn fetch_data_from_overpass(
    bbox: LLBBox,
    debug: bool,
    download_method: &str,
    save_file: Option<&str>,
) -> Result<OsmData, Box<dyn std::error::Error>> {
    println!("{} Fetching data...", "[1/7]".bold());
    emit_gui_progress_update(1.0, "Fetching data...");

    // List of Overpass API servers
    let api_servers: Vec<&str> = vec![
        "https://overpass-api.de/api/interpreter",
        "https://lz4.overpass-api.de/api/interpreter",
        "https://z.overpass-api.de/api/interpreter",
        //"https://overpass.kumi.systems/api/interpreter", // This server is not reliable anymore
        //"https://overpass.private.coffee/api/interpreter", // This server is not reliable anymore
    ];
    let fallback_api_servers: Vec<&str> =
        vec!["https://maps.mail.ru/osm/tools/overpass/api/interpreter"];
    let raw_data = fetch_bbox_adaptive(
        bbox,
        download_method,
        &api_servers,
        &fallback_api_servers,
        0,
        false,
    )?;
    let response = serde_json::to_string(&raw_data)?;

    if let Some(save_file) = save_file {
        let mut file: File = File::create(save_file)?;
        file.write_all(response.as_bytes())?;
        println!("API response saved to: {save_file}");
    }

    let mut deserializer = serde_json::Deserializer::from_reader(Cursor::new(response.as_bytes()));
    let data: OsmData = OsmData::deserialize(&mut deserializer)?;

    if data.is_empty() {
        if let Some(remark) = data.remark.as_deref() {
            if remark.contains("runtime error") && remark.contains("out of memory") {
                eprintln!("{}", "Error! The query ran out of memory on the Overpass API server. Try using a smaller area.".red().bold());
                emit_gui_error("Try using a smaller area.");
            } else {
                eprintln!("{}", format!("Error! API returned: {remark}").red().bold());
                emit_gui_error(&format!("API returned: {remark}"));
            }
        } else {
            eprintln!(
                "{}",
                "Error! API returned no data. Please try again!"
                    .red()
                    .bold()
            );
            emit_gui_error("API returned no data. Please try again!");
        }

        if debug {
            println!("Additional debug information: {data:?}");
        }

        if !is_running_with_gui() {
            std::process::exit(1);
        } else {
            return Err("Data fetch failed".into());
        }
    }

    emit_gui_progress_update(5.0, "");

    Ok(data)
}

/// Fetches a short area name using Nominatim for the given lat/lon
pub fn fetch_area_name(lat: f64, lon: f64) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let client = Client::builder().timeout(Duration::from_secs(20)).build()?;

    let url = format!("https://nominatim.openstreetmap.org/reverse?format=jsonv2&lat={lat}&lon={lon}&addressdetails=1");

    let resp = client.get(&url).header("User-Agent", "arnis-rust").send()?;

    if !resp.status().is_success() {
        return Ok(None);
    }

    let json: Value = resp.json()?;

    if let Some(address) = json.get("address") {
        let fields = ["city", "town", "village", "county", "borough", "suburb"];
        for field in fields.iter() {
            if let Some(name) = address.get(*field).and_then(|v| v.as_str()) {
                let mut name_str = name.to_string();

                // Remove "City of " prefix
                if name_str.to_lowercase().starts_with("city of ") {
                    name_str = name_str[name_str.find(" of ").unwrap() + 4..].to_string();
                }

                return Ok(Some(name_str));
            }
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overpass_query_omits_bare_way_union() {
        let bbox = LLBBox::new(54.6, 25.2, 54.7, 25.3).unwrap();
        let query = build_overpass_query(bbox);

        assert!(query.contains("way[\"place\"]"));
        assert!(!query.contains("\n        way;\n"));
    }

    #[test]
    fn initial_tiling_splits_large_bboxes() {
        let bbox = LLBBox::new(41.30, 2.05, 41.48, 2.26).unwrap();
        let tiles = initial_tiles_for_bbox(bbox);

        assert!(tiles.len() > 1);
        assert_eq!(tiles.first().unwrap().min(), bbox.min());
        assert_eq!(tiles.last().unwrap().max(), bbox.max());
    }

    #[test]
    fn merge_raw_responses_deduplicates_by_type_and_id() {
        let merged = merge_raw_overpass_responses(vec![
            RawOverpassResponse {
                elements: vec![
                    serde_json::json!({"type":"node","id":1,"lat":1.0,"lon":2.0}),
                    serde_json::json!({"type":"way","id":7,"nodes":[1,2]}),
                ],
                remark: None,
            },
            RawOverpassResponse {
                elements: vec![
                    serde_json::json!({"type":"node","id":1,"lat":1.0,"lon":2.0}),
                    serde_json::json!({"type":"relation","id":9,"members":[]}),
                ],
                remark: Some("runtime error: out of memory".to_string()),
            },
        ]);

        assert_eq!(merged.elements.len(), 3);
        assert!(merged.remark.unwrap().contains("out of memory"));
    }

    #[test]
    fn split_is_suggested_for_oom_and_timeout() {
        assert!(overpass_error_suggests_split(
            "Request timed out. Try selecting a smaller area."
        ));
        assert!(overpass_error_suggests_split(
            "Error! Received response code: 504"
        ));
        assert!(overpass_remark_suggests_split(
            "runtime error: Query run out of memory"
        ));
    }
}
