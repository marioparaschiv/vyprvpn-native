//! Where each VyprVPN location's server stands, for marking it on the globe. Embedded, so the globe
//! never waits on the network.

use crate::Coordinates;

/// Spoke id, latitude and longitude of each location's server city, as the locations endpoint
/// reports them.
const CITIES: &[(&str, f32, f32)] = &[
    ("uk1", 51.5049, -0.114),
    ("be1", 50.85, 4.35),
    ("eu1", 52.3613, 4.9081),
    ("fr1", 48.8457, 2.3483),
    ("ie1", 53.3483, -6.268),
    ("lu1", 49.6093, 6.1335),
    ("de1", 50.1055, 8.6765),
    ("ch1", 46.9507, 7.4389),
    ("li1", 47.1675, 9.5105),
    ("dk1", 55.6711, 12.5642),
    ("cz1", 50.0833, 14.4167),
    ("no1", 59.913, 10.7512),
    ("si1", 46.0569, 14.5058),
    ("at1", 48.2083, 16.7371),
    ("es1", 40.4, -3.6833),
    ("sk1", 48.1486, 17.1077),
    ("se1", 59.3216, 18.067),
    ("it1", 41.9, 12.5),
    ("pl1", 52.23, 21.0167),
    ("pt1", 38.7138, -9.1394),
    ("lv1", 56.9496, 24.1052),
    ("dz1", 36.7538, 3.0588),
    ("lt1", 54.6833, 25.2833),
    ("fi1", 60.1708, 24.9375),
    ("is1", 64.1333, -21.9333),
    ("bg1", 42.6977, 23.3219),
    ("ro1", 44.4246, 26.109),
    ("ua1", 50.4501, 30.5234),
    ("gr1", 37.9838, 23.7275),
    ("ru1", 55.7448, 37.6138),
    ("tr1", 41.0112, 28.974),
    ("eg1", 30.0444, 31.2357),
    ("il1", 32.0667, 34.8),
    ("sa1", 24.6333, 46.7167),
    ("bh1", 26.2167, 50.5833),
    ("qa1", 25.2867, 51.5333),
    ("ae1", 25.2015, 55.2708),
    ("us5", 40.7127, -74.0059),
    ("ca1", 43.6481, -79.4042),
    ("us2", 38.8813, -77.0224),
    ("pk1", 24.8615, 67.0099),
    ("us6", 41.8819, -87.6278),
    ("us4", 25.7216, -80.2793),
    ("in1", 18.975, 72.8258),
    ("us8", 47.6143, -122.3396),
    ("us3", 30.2565, -97.6849),
    ("pa1", 8.9833, -79.5167),
    ("co1", 4.5981, -74.0758),
    ("mv1", 4.1755, 73.5093),
    ("us7", 37.7833, -122.4167),
    ("cr1", 9.9333, -84.0833),
    ("us1", 34.0459, -118.2321),
    ("sv1", 13.6929, -89.2182),
    ("kr1", 37.5833, 127.0),
    ("mx1", 19.4328, -99.1333),
    ("vn1", 21.0333, 105.85),
    ("br1", -23.55, -46.6333),
    ("th1", 13.75, 100.4667),
    ("jp1", 35.6833, 139.7667),
    ("mo1", 22.1987, 113.5439),
    ("hk1", 22.3169, 114.1205),
    ("tw1", 22.95, 120.0),
    ("my1", 3.1597, 101.7),
    ("ph1", 14.5833, 121.0),
    ("sg1", 1.353, 103.819),
    ("uy1", -34.9011, -56.1645),
    ("ar1", -34.6033, -58.3817),
    ("id1", -6.2, 106.8),
    ("mh1", 7.1164, 171.1858),
    ("au3", -31.9522, 115.8589),
    ("au2", -37.8136, 144.9631),
    ("au1", -33.86, 151.2111),
    ("nz1", -36.8404, 174.7399),
];

/// The locations endpoint's country code, latitude and longitude of each country's centre.
const COUNTRIES: &[(&str, f32, f32)] = &[
    ("ALG", 28.0, 2.6),
    ("ARG", -38.4, -63.6),
    ("AUS", -25.3, 133.8),
    ("AUT", 47.6, 14.1),
    ("BEL", 50.6, 4.6),
    ("BRA", -10.8, -52.9),
    ("BRN", 26.05, 50.55),
    ("BUL", 42.7, 25.3),
    ("CAN", 56.0, -96.0),
    ("COL", 4.0, -73.0),
    ("CRC", 9.9, -84.1),
    ("CZE", 49.8, 15.5),
    ("DEN", 56.0, 9.5),
    ("EGY", 26.7, 30.0),
    ("ESA", 13.7, -88.9),
    ("ESP", 40.2, -3.6),
    ("FIN", 64.0, 26.0),
    ("FRA", 46.6, 2.4),
    ("GBR", 54.0, -2.5),
    ("GER", 51.2, 10.4),
    ("GRE", 39.1, 22.0),
    ("HKG", 22.35, 114.15),
    ("INA", -2.5, 118.0),
    ("IND", 22.9, 79.0),
    ("IRL", 53.2, -8.0),
    ("ISL", 64.9, -18.6),
    ("ISR", 31.4, 35.0),
    ("ITA", 42.8, 12.5),
    ("JPN", 36.2, 138.3),
    ("KOR", 36.4, 127.9),
    ("KSA", 24.0, 45.0),
    ("LAT", 56.9, 24.6),
    ("LIE", 47.15, 9.55),
    ("LTU", 55.2, 23.9),
    ("LUX", 49.8, 6.1),
    ("MAC", 22.17, 113.55),
    ("MAS", 4.0, 102.0),
    ("MDV", 3.2, 73.2),
    ("MEX", 23.6, -102.5),
    ("MHL", 7.1, 171.2),
    ("NED", 52.2, 5.5),
    ("NOR", 61.5, 9.0),
    ("NZL", -41.5, 172.5),
    ("PAK", 30.0, 69.4),
    ("PAN", 8.5, -80.1),
    ("PHI", 12.9, 121.8),
    ("POL", 52.1, 19.4),
    ("POR", 39.6, -8.0),
    ("QAT", 25.3, 51.2),
    ("ROU", 45.9, 25.0),
    ("RUS", 61.5, 105.0),
    ("SIN", 1.35, 103.82),
    ("SLO", 46.1, 14.8),
    ("SUI", 46.8, 8.2),
    ("SVK", 48.7, 19.7),
    ("SWE", 62.0, 15.0),
    ("THA", 15.1, 101.0),
    ("TPE", 23.7, 121.0),
    ("TUR", 39.0, 35.2),
    ("UAE", 23.9, 54.3),
    ("UKR", 49.0, 31.4),
    ("URU", -32.8, -56.0),
    ("USA", 39.8, -98.6),
    ("VIE", 16.4, 107.5),
];

/// Where a location's server stands: its city, else the centre of its country, else unknown.
pub fn place(spoke_id: &str, country_code: Option<&str>) -> Option<Coordinates> {
    let find = |table: &[(&str, f32, f32)], key: &str| {
        table
            .iter()
            .find(|(id, _, _)| *id == key)
            .map(|&(_, lat, lon)| Coordinates { lon, lat })
    };

    find(CITIES, spoke_id).or_else(|| find(COUNTRIES, country_code?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// South, north, west and east edges of each country the recorded locations are in, in
    /// degrees; the United States' are the contiguous states'.
    const BOUNDS: &[(&str, [f32; 4])] = &[
        ("ALG", [19.0, 37.1, -8.7, 12.0]),
        ("ARG", [-55.1, -21.8, -73.6, -53.6]),
        ("AUS", [-43.7, -10.6, 113.2, 153.7]),
        ("AUT", [46.4, 49.0, 9.5, 17.2]),
        ("BEL", [49.5, 51.5, 2.5, 6.4]),
        ("BRA", [-33.8, 5.3, -74.0, -34.8]),
        ("BRN", [25.8, 26.3, 50.4, 50.8]),
        ("BUL", [41.2, 44.2, 22.4, 28.6]),
        ("CAN", [41.7, 83.1, -141.0, -52.6]),
        ("COL", [-4.2, 12.5, -79.0, -66.9]),
        ("CRC", [8.0, 11.2, -86.0, -82.5]),
        ("CZE", [48.5, 51.1, 12.1, 18.9]),
        ("DEN", [54.5, 57.8, 8.0, 15.2]),
        ("EGY", [22.0, 31.7, 24.7, 36.9]),
        ("ESA", [13.1, 14.5, -90.1, -87.7]),
        ("ESP", [36.0, 43.8, -9.3, 3.3]),
        ("FIN", [59.8, 70.1, 20.5, 31.6]),
        ("FRA", [41.3, 51.1, -5.2, 9.6]),
        ("GBR", [49.9, 60.9, -8.7, 1.8]),
        ("GER", [47.3, 55.1, 5.9, 15.1]),
        ("GRE", [34.8, 41.8, 19.4, 29.7]),
        ("HKG", [22.15, 22.56, 113.83, 114.44]),
        ("INA", [-11.0, 6.1, 95.0, 141.0]),
        ("IND", [6.7, 35.5, 68.1, 97.4]),
        ("IRL", [51.4, 55.4, -10.5, -6.0]),
        ("ISL", [63.3, 66.6, -24.6, -13.5]),
        ("ISR", [29.5, 33.3, 34.3, 35.9]),
        ("ITA", [36.6, 47.1, 6.6, 18.5]),
        ("JPN", [24.0, 45.6, 122.9, 154.0]),
        ("KOR", [33.1, 38.6, 124.6, 131.9]),
        ("KSA", [16.4, 32.2, 34.5, 55.7]),
        ("LAT", [55.7, 58.1, 21.0, 28.3]),
        ("LIE", [47.0, 47.3, 9.47, 9.64]),
        ("LTU", [53.9, 56.5, 21.0, 26.9]),
        ("LUX", [49.4, 50.2, 5.7, 6.6]),
        ("MAC", [22.1, 22.22, 113.52, 113.6]),
        ("MAS", [0.85, 7.4, 99.6, 119.3]),
        ("MDV", [-0.7, 7.1, 72.6, 73.8]),
        ("MEX", [14.5, 32.7, -118.4, -86.7]),
        ("MHL", [4.5, 14.7, 160.8, 172.2]),
        ("NED", [50.7, 53.6, 3.3, 7.3]),
        ("NOR", [57.9, 71.2, 4.6, 31.1]),
        ("NZL", [-47.3, -34.4, 166.4, 178.6]),
        ("PAK", [23.6, 37.1, 60.9, 77.8]),
        ("PAN", [7.2, 9.7, -83.1, -77.2]),
        ("PHI", [4.6, 21.1, 116.9, 126.6]),
        ("POL", [49.0, 54.9, 14.1, 24.2]),
        ("POR", [36.9, 42.2, -9.5, -6.2]),
        ("QAT", [24.5, 26.2, 50.7, 51.7]),
        ("ROU", [43.6, 48.3, 20.2, 29.7]),
        ("RUS", [41.2, 81.9, 19.6, 180.0]),
        ("SIN", [1.15, 1.48, 103.6, 104.1]),
        ("SLO", [45.4, 46.9, 13.4, 16.6]),
        ("SUI", [45.8, 47.9, 5.9, 10.5]),
        ("SVK", [47.7, 49.6, 16.8, 22.6]),
        ("SWE", [55.3, 69.1, 11.1, 24.2]),
        ("THA", [5.6, 20.5, 97.3, 105.6]),
        ("TPE", [21.9, 25.3, 119.3, 122.0]),
        ("TUR", [35.8, 42.1, 26.0, 44.8]),
        ("UAE", [22.6, 26.1, 51.5, 56.4]),
        ("UKR", [44.4, 52.4, 22.1, 40.2]),
        ("URU", [-35.0, -30.1, -58.5, -53.1]),
        ("USA", [24.5, 49.4, -125.0, -66.9]),
        ("VIE", [8.6, 23.4, 102.1, 109.5]),
    ];

    #[derive(serde::Deserialize)]
    struct Recorded {
        locations: Vec<Location>,
    }

    #[derive(serde::Deserialize)]
    struct Location {
        spoke_id: String,
        country_code: String,
        latitude: f32,
        longitude: f32,
    }

    fn recorded() -> Vec<Location> {
        let recorded: Recorded =
            serde_json::from_str(include_str!("../../vypr-api/tests/fixtures/locations.json"))
                .unwrap();

        recorded.locations
    }

    fn assert_within(place: Coordinates, country_code: &str) {
        let [south, north, west, east] = BOUNDS
            .iter()
            .find(|(code, _)| *code == country_code)
            .unwrap_or_else(|| panic!("no bounds for {country_code}"))
            .1;

        assert!(
            (south..=north).contains(&place.lat) && (west..=east).contains(&place.lon),
            "{place:?} is outside {country_code}"
        );
    }

    #[test]
    fn every_recorded_location_stands_in_its_country_where_the_endpoint_says() {
        let locations = recorded();
        assert_eq!(locations.len(), CITIES.len());

        for location in locations {
            let place = place(&location.spoke_id, Some(&location.country_code))
                .unwrap_or_else(|| panic!("no place for {}", location.spoke_id));

            assert_within(place, &location.country_code);
            assert!(
                (place.lat - location.latitude).abs() < 0.001
                    && (place.lon - location.longitude).abs() < 0.001,
                "{}: {place:?}",
                location.spoke_id
            );
        }
    }

    #[test]
    fn unknown_locations_fall_back_to_their_country_centre() {
        assert_eq!(COUNTRIES.len(), BOUNDS.len());
        for &(country_code, _, _) in COUNTRIES {
            let centre = place("new1", Some(country_code)).unwrap();
            assert_within(centre, country_code);
        }

        assert_eq!(
            place("de2", Some("GER")),
            Some(Coordinates {
                lon: 10.4,
                lat: 51.2
            })
        );
        assert_eq!(place("xx1", Some("XXX")), None);
        assert_eq!(place("xx1", None), None);
    }
}
