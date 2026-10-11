use std::collections::BTreeMap;

use vypr_api::Location;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub name: String,
    pub countries: Vec<Country>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Country {
    pub name: String,
    pub country_code: String,
    /// One entry for a country with a single location; the menu shows a City submenu otherwise.
    pub cities: Vec<City>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct City {
    pub name: String,
    pub spoke_id: String,
}

/// Groups locations by region and country, each level sorted by name. Takes a slice or
/// [`filter`]'s matches.
pub fn group<'a>(locations: impl IntoIterator<Item = &'a Location>) -> Vec<Region> {
    let mut regions: BTreeMap<&str, BTreeMap<&str, Vec<&Location>>> = BTreeMap::new();
    for location in locations {
        regions
            .entry(&location.region)
            .or_default()
            .entry(&location.country)
            .or_default()
            .push(location);
    }

    regions
        .into_iter()
        .map(|(region, countries)| Region {
            name: region.to_owned(),
            countries: countries
                .into_iter()
                .map(|(country, locations)| self::country(country, &locations))
                .collect(),
        })
        .collect()
}

fn country(name: &str, locations: &[&Location]) -> Country {
    let mut cities: Vec<City> = locations
        .iter()
        .map(|location| City {
            name: city_name(location).to_owned(),
            spoke_id: location.spoke_id.clone(),
        })
        .collect();
    cities.sort_by(|a, b| a.name.cmp(&b.name));

    Country {
        name: name.to_owned(),
        country_code: locations[0].country_code.clone(),
        cities,
    }
}

/// "Australia - Perth" is Perth. The prefix is not always the country: "USA - Miami" is in
/// "United States".
fn city_name(location: &Location) -> &str {
    location
        .name
        .split_once(" - ")
        .map_or(&location.name, |(_, city)| city)
}

/// The locations whose name, country, country code, region or spoke id has a word starting with
/// `query`, ignoring case, in their original order. A blank query matches every location.
pub fn filter<'a>(locations: &'a [Location], query: &str) -> Vec<&'a Location> {
    let query = query.trim().to_lowercase();

    locations
        .iter()
        .filter(|location| {
            [
                &location.name,
                &location.country,
                &location.country_code,
                &location.region,
                &location.spoke_id,
            ]
            .into_iter()
            .any(|field| has_word_starting_with(&field.to_lowercase(), &query))
        })
        .collect()
}

/// Matching only at word starts keeps "ger" on Germany instead of also hitting Algeria.
fn has_word_starting_with(text: &str, prefix: &str) -> bool {
    text.match_indices(prefix).any(|(index, _)| {
        text[..index]
            .chars()
            .next_back()
            .is_none_or(|before| !before.is_alphanumeric())
    })
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    const LOCATIONS: &str = include_str!("../../vypr-api/tests/fixtures/locations.json");

    fn live_locations() -> Vec<Location> {
        #[derive(Deserialize)]
        struct Body {
            locations: Vec<Location>,
        }

        serde_json::from_str::<Body>(LOCATIONS).unwrap().locations
    }

    fn names<T>(items: &[T], name: impl Fn(&T) -> &str) -> Vec<&str> {
        items.iter().map(name).collect()
    }

    #[test]
    fn groups_by_region_country_and_city() {
        let regions = group(&live_locations());

        assert_eq!(
            names(&regions, |region| &region.name),
            [
                "Africa",
                "Asia",
                "Europe",
                "North America",
                "Oceania",
                "South America"
            ]
        );

        let oceania = &regions[4];
        let australia = &oceania.countries[0];
        assert_eq!(australia.name, "Australia");
        assert_eq!(australia.country_code, "AUS");
        assert_eq!(
            australia.cities,
            [
                City {
                    name: "Melbourne".into(),
                    spoke_id: "au2".into()
                },
                City {
                    name: "Perth".into(),
                    spoke_id: "au3".into()
                },
                City {
                    name: "Sydney".into(),
                    spoke_id: "au1".into()
                },
            ]
        );

        let united_states = regions[3]
            .countries
            .iter()
            .find(|country| country.name == "United States")
            .unwrap();
        assert_eq!(
            names(&united_states.cities, |city| &city.name),
            [
                "Austin",
                "Chicago",
                "Los Angeles",
                "Miami",
                "New York",
                "San Francisco",
                "Seattle",
                "Washington"
            ]
        );
    }

    #[test]
    fn single_location_countries_have_one_entry() {
        let regions = group(&live_locations());
        let europe = &regions[2];

        let germany = europe
            .countries
            .iter()
            .find(|country| country.name == "Germany")
            .unwrap();
        assert_eq!(germany.country_code, "GER");
        assert_eq!(germany.cities.len(), 1);

        let country_names = names(&europe.countries, |country| &country.name);
        let mut sorted = country_names.clone();
        sorted.sort_unstable();
        assert_eq!(country_names, sorted);
    }

    #[test]
    fn keeps_every_location() {
        let locations = live_locations();
        let regions = group(&locations);

        let mut spoke_ids: Vec<&str> = regions
            .iter()
            .flat_map(|region| &region.countries)
            .flat_map(|country| &country.cities)
            .map(|city| city.spoke_id.as_str())
            .collect();
        spoke_ids.sort_unstable();
        let mut expected: Vec<&str> = locations.iter().map(|l| l.spoke_id.as_str()).collect();
        expected.sort_unstable();

        assert_eq!(locations.len(), 73);
        assert_eq!(spoke_ids, expected);
    }

    fn filtered_spoke_ids<'a>(locations: &'a [Location], query: &str) -> Vec<&'a str> {
        filter(locations, query)
            .into_iter()
            .map(|location| location.spoke_id.as_str())
            .collect()
    }

    #[test]
    fn filters_by_name_country_code_or_spoke_id() {
        let locations = live_locations();

        assert_eq!(filtered_spoke_ids(&locations, "perth"), ["au3"]);
        assert_eq!(filtered_spoke_ids(&locations, "GER"), ["de1"]);
        assert_eq!(filtered_spoke_ids(&locations, "de1"), ["de1"]);
    }

    #[test]
    fn filters_by_country_and_region_in_original_order() {
        let locations = live_locations();

        assert_eq!(
            filtered_spoke_ids(&locations, "australia"),
            ["au3", "au2", "au1"]
        );
        assert_eq!(filtered_spoke_ids(&locations, "Oceania").len(), 4);
        assert_eq!(filtered_spoke_ids(&locations, "new york"), ["us5"]);
    }

    #[test]
    fn blank_query_keeps_everything() {
        let locations = live_locations();

        assert_eq!(filter(&locations, "  ").len(), locations.len());
    }

    #[test]
    fn groups_filtered_locations() {
        let locations = live_locations();
        let regions = group(filter(&locations, "australia"));

        assert_eq!(names(&regions, |region| &region.name), ["Oceania"]);
        assert_eq!(regions[0].countries[0].cities.len(), 3);
    }
}
