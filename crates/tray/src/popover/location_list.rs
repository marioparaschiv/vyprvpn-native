use std::path::PathBuf;

use gpui_kit::component::list::{ListDelegate, ListItem, ListState};
use gpui_kit::component::{ActiveTheme, IconName, IndexPath, StyledExt, h_flex};
use gpui_kit::*;
use ipc::Location;

use super::flag;

/// The subpage's location list: regions as sections, one row per location, narrowed by the
/// search box.
pub struct LocationList {
    locations: Vec<Location>,
    query: String,
    regions: Vec<Region>,
    /// The location Connect goes to, marked with a check.
    chosen: Option<String>,
    flag_dir: Option<PathBuf>,
}

struct Region {
    name: SharedString,
    rows: Vec<Row>,
}

struct Row {
    spoke_id: SharedString,
    country_code: String,
    country: SharedString,
    /// Shown only for a country with more than one location.
    city: Option<SharedString>,
}

impl LocationList {
    pub fn new(flag_dir: Option<PathBuf>) -> Self {
        Self {
            locations: Vec::new(),
            query: String::new(),
            regions: Vec::new(),
            chosen: None,
            flag_dir,
        }
    }

    /// Takes effect with the next search, which the owner runs after this.
    pub fn set_locations(&mut self, locations: Vec<Location>) {
        self.locations = locations;
    }

    /// The text the list was last narrowed by.
    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn set_chosen(&mut self, spoke_id: Option<&str>) {
        self.chosen = spoke_id.map(str::to_owned);
    }

    pub fn spoke_id(&self, index: IndexPath) -> Option<&str> {
        let row = self.regions.get(index.section)?.rows.get(index.row)?;

        Some(&row.spoke_id)
    }

    pub fn index_of(&self, spoke_id: &str) -> Option<IndexPath> {
        self.regions
            .iter()
            .enumerate()
            .find_map(|(section, region)| {
                let row = region
                    .rows
                    .iter()
                    .position(|row| row.spoke_id == spoke_id)?;
                Some(IndexPath::new(row).section(section))
            })
    }

    fn search(&mut self, query: &str) {
        query.clone_into(&mut self.query);
        self.regions = locations::group(locations::filter(&self.locations, query))
            .into_iter()
            .map(|region| Region {
                name: region.name.into(),
                rows: region
                    .countries
                    .into_iter()
                    .flat_map(|country| {
                        let several = country.cities.len() > 1;
                        let country_name = SharedString::from(country.name);

                        country.cities.into_iter().map(move |city| Row {
                            spoke_id: city.spoke_id.into(),
                            country_code: country.country_code.clone(),
                            country: country_name.clone(),
                            city: several.then(|| city.name.into()),
                        })
                    })
                    .collect(),
            })
            .collect();
    }
}

impl ListDelegate for LocationList {
    type Item = ListItem;

    fn perform_search(
        &mut self,
        query: &str,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        self.search(query);
        Task::ready(())
    }

    fn set_selected_index(
        &mut self,
        _: Option<IndexPath>,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) {
    }

    fn sections_count(&self, _: &App) -> usize {
        self.regions.len()
    }

    fn items_count(&self, section: usize, _: &App) -> usize {
        self.regions
            .get(section)
            .map_or(0, |region| region.rows.len())
    }

    fn render_item(
        &mut self,
        index: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let row = self.regions.get(index.section)?.rows.get(index.row)?;
        let chosen = self.chosen.as_deref() == Some(row.spoke_id.as_ref());

        Some(
            ListItem::new(ElementId::Name(row.spoke_id.clone()))
                .check_icon(IconName::Check)
                .confirmed(chosen)
                .child(
                    h_flex()
                        .gap_2()
                        .min_w_0()
                        .child(flag(self.flag_dir.as_deref(), &row.country_code, cx))
                        .child(div().text_ellipsis().child(row.country.clone()))
                        .children(row.city.clone().map(|city| {
                            div()
                                .text_ellipsis()
                                .text_color(cx.theme().muted_foreground)
                                .child(city)
                        })),
                ),
        )
    }

    fn render_section_header(
        &mut self,
        section: usize,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<impl IntoElement> {
        let region = self.regions.get(section)?;

        Some(
            div()
                .px_3()
                .pt_3()
                .pb_1()
                .text_xs()
                .font_medium()
                .text_color(cx.theme().muted_foreground)
                .child(region.name.clone()),
        )
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        h_flex()
            .size_full()
            .justify_center()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child("No matching locations")
    }
}
