//! Rasterises Natural Earth's 1:110m land polygons (public domain) into the globe crate's land mask.

use std::fs;
use std::path::Path;

use globe::{MASK_COLUMNS, MASK_ROWS};
use serde::Deserialize;

use crate::wireguard::{download, verify_sha256};
use crate::{Error, io_error};

const GEOJSON_URL: &str = "https://raw.githubusercontent.com/nvkelso/natural-earth-vector/v5.1.2/geojson/ne_110m_land.geojson";
const GEOJSON_NAME: &str = "ne_110m_land.geojson";
const GEOJSON_SHA256: &str = "9e0729ee253ca7d7a5c4ae9395fb1902264c5377c52e224d13dd85010e2835d9";

type Ring = Vec<[f64; 2]>;

#[derive(Deserialize)]
struct FeatureCollection {
    features: Vec<Feature>,
}

#[derive(Deserialize)]
struct Feature {
    geometry: Geometry,
}

#[derive(Deserialize)]
#[serde(tag = "type", content = "coordinates")]
enum Geometry {
    Polygon(Vec<Ring>),
    MultiPolygon(Vec<Vec<Ring>>),
}

pub fn generate(mask_path: &Path) -> Result<(), Error> {
    let geojson = download(GEOJSON_URL)?;
    verify_sha256(&geojson, GEOJSON_SHA256, Path::new(GEOJSON_NAME))?;
    let land: FeatureCollection = serde_json::from_slice(&geojson).map_err(Error::LandData)?;

    let rings: Vec<Ring> = land
        .features
        .into_iter()
        .flat_map(|feature| match feature.geometry {
            Geometry::Polygon(rings) => rings,
            Geometry::MultiPolygon(polygons) => polygons.into_iter().flatten().collect(),
        })
        .collect();

    fs::write(mask_path, rasterise(&rings)).map_err(io_error("writing", mask_path))?;
    println!("land mask written: {}", mask_path.display());

    Ok(())
}

/// One bit per cell, set where the cell's centre is inside an odd number of rings, so holes stay
/// sea. Rows run north to south, the most significant bit first.
fn rasterise(rings: &[Ring]) -> Vec<u8> {
    let mut mask = vec![0u8; MASK_COLUMNS * MASK_ROWS / 8];
    let cell = 360.0 / MASK_COLUMNS as f64;

    for row in 0..MASK_ROWS {
        let lat = 90.0 - (row as f64 + 0.5) * cell;
        let crossings: Vec<f64> = rings
            .iter()
            .flat_map(|ring| ring.windows(2))
            .filter(|edge| (edge[0][1] > lat) != (edge[1][1] > lat))
            .map(|edge| {
                let [[x0, y0], [x1, y1]] = [edge[0], edge[1]];
                x0 + (lat - y0) / (y1 - y0) * (x1 - x0)
            })
            .collect();

        for column in 0..MASK_COLUMNS {
            let lon = -180.0 + (column as f64 + 0.5) * cell;
            if crossings.iter().filter(|&&x| x < lon).count() % 2 == 1 {
                let bit = row * MASK_COLUMNS + column;
                mask[bit / 8] |= 0x80 >> (bit % 8);
            }
        }
    }

    mask
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_set(mask: &[u8], lon: f64, lat: f64) -> bool {
        let column = ((lon + 180.0) / 360.0 * MASK_COLUMNS as f64) as usize;
        let row = ((90.0 - lat) / 180.0 * MASK_ROWS as f64) as usize;
        let bit = row * MASK_COLUMNS + column;

        mask[bit / 8] & (0x80 >> (bit % 8)) != 0
    }

    #[test]
    fn fills_rings_and_leaves_holes() {
        let square = |size: f64| -> Ring {
            vec![
                [-size, -size],
                [size, -size],
                [size, size],
                [-size, size],
                [-size, -size],
            ]
        };
        let mask = rasterise(&[square(20.0), square(5.0)]);

        assert!(is_set(&mask, 10.0, 10.0));
        assert!(is_set(&mask, -19.9, 0.0));
        assert!(!is_set(&mask, 0.0, 0.0), "the inner ring is a hole");
        assert!(!is_set(&mask, 30.0, 0.0));
        assert!(!is_set(&mask, 0.0, -30.0));
    }
}
