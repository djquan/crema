use std::collections::{BTreeMap, HashSet};

use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Element, Length, Padding};

use crema_catalog::models::Photo;

use crate::app::Message;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DateFilter {
    All,
    Year(u16),
    Month(u16, u8),
    Day(u16, u8, u8),
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DateExpansionKey {
    Year(u16),
    Month(u16, u8),
}

impl DateFilter {
    pub fn matches(&self, photo: &Photo) -> bool {
        match self {
            DateFilter::All => true,
            DateFilter::Unknown => parse_date(photo.date_taken.as_deref()).is_none(),
            DateFilter::Year(y) => {
                parse_date(photo.date_taken.as_deref()).is_some_and(|(py, _, _)| py == *y)
            }
            DateFilter::Month(y, m) => parse_date(photo.date_taken.as_deref())
                .is_some_and(|(py, pm, _)| py == *y && pm == *m),
            DateFilter::Day(y, m, d) => parse_date(photo.date_taken.as_deref())
                .is_some_and(|(py, pm, pd)| py == *y && pm == *m && pd == *d),
        }
    }
}

pub fn parse_date(s: Option<&str>) -> Option<(u16, u8, u8)> {
    let s = s?;
    if s.len() < 10 {
        return None;
    }
    let date_part = &s[..10];
    let parts: Vec<&str> = date_part.split('-').collect();
    if parts.len() < 3 {
        return None;
    }
    let year: u16 = parts[0].parse().ok()?;
    let month: u8 = parts[1].parse().ok()?;
    let day: u8 = parts[2].parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some((year, month, day))
}

struct DateTree {
    total: usize,
    years: Vec<YearEntry>,
    unknown_count: usize,
}

struct YearEntry {
    year: u16,
    count: usize,
    months: Vec<MonthEntry>,
}

struct MonthEntry {
    month: u8,
    count: usize,
    days: Vec<DayEntry>,
}

struct DayEntry {
    day: u8,
    count: usize,
}

fn build_date_tree(photos: &[Photo]) -> DateTree {
    let mut map: BTreeMap<u16, BTreeMap<u8, BTreeMap<u8, usize>>> = BTreeMap::new();
    let mut unknown_count = 0;

    for photo in photos {
        match parse_date(photo.date_taken.as_deref()) {
            Some((y, m, d)) => {
                *map.entry(y)
                    .or_default()
                    .entry(m)
                    .or_default()
                    .entry(d)
                    .or_default() += 1;
            }
            None => unknown_count += 1,
        }
    }

    let mut years: Vec<YearEntry> = map
        .into_iter()
        .map(|(year, months_map)| {
            let mut year_count = 0;
            let months: Vec<MonthEntry> = months_map
                .into_iter()
                .map(|(month, days_map)| {
                    let mut month_count = 0;
                    let days: Vec<DayEntry> = days_map
                        .into_iter()
                        .map(|(day, count)| {
                            month_count += count;
                            DayEntry { day, count }
                        })
                        .collect();
                    year_count += month_count;
                    MonthEntry {
                        month,
                        count: month_count,
                        days,
                    }
                })
                .collect();
            YearEntry {
                year,
                count: year_count,
                months,
            }
        })
        .collect();

    years.reverse();

    DateTree {
        total: photos.len(),
        years,
        unknown_count,
    }
}

const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn month_name(m: u8) -> &'static str {
    MONTH_NAMES
        .get((m as usize).wrapping_sub(1))
        .unwrap_or(&"???")
}

const SIDEBAR_WIDTH: f32 = 200.0;

pub fn view<'a>(
    photos: &[Photo],
    active_filter: &DateFilter,
    expanded: &HashSet<DateExpansionKey>,
) -> Element<'a, Message> {
    let tree = build_date_tree(photos);
    let mut items: Vec<Element<'a, Message>> = Vec::new();

    items.push(filter_button(
        format!("All ({})", tree.total),
        DateFilter::All,
        active_filter,
        0,
    ));

    for year_entry in &tree.years {
        let year_key = DateExpansionKey::Year(year_entry.year);
        let is_expanded = expanded.contains(&year_key);
        let arrow = if is_expanded { "v" } else { ">" };

        items.push(
            row![
                arrow_button(arrow, year_key),
                filter_button(
                    format!("{} ({})", year_entry.year, year_entry.count),
                    DateFilter::Year(year_entry.year),
                    active_filter,
                    0,
                ),
            ]
            .spacing(0)
            .into(),
        );

        if is_expanded {
            for month_entry in &year_entry.months {
                let month_key = DateExpansionKey::Month(year_entry.year, month_entry.month);
                let is_month_expanded = expanded.contains(&month_key);
                let arrow = if is_month_expanded { "v" } else { ">" };

                items.push(
                    row![
                        Space::new().width(16),
                        arrow_button(arrow, month_key),
                        filter_button(
                            format!("{} ({})", month_name(month_entry.month), month_entry.count),
                            DateFilter::Month(year_entry.year, month_entry.month),
                            active_filter,
                            0,
                        ),
                    ]
                    .spacing(0)
                    .into(),
                );

                if is_month_expanded {
                    for day_entry in &month_entry.days {
                        items.push(filter_button(
                            format!("{} ({})", day_entry.day, day_entry.count),
                            DateFilter::Day(year_entry.year, month_entry.month, day_entry.day),
                            active_filter,
                            48,
                        ));
                    }
                }
            }
        }
    }

    if tree.unknown_count > 0 {
        items.push(filter_button(
            format!("Unknown ({})", tree.unknown_count),
            DateFilter::Unknown,
            active_filter,
            0,
        ));
    }

    container(scrollable(column(items).spacing(2).padding(8)).height(Length::Fill))
        .width(SIDEBAR_WIDTH)
        .height(Length::Fill)
        .into()
}

fn filter_button<'a>(
    label: String,
    filter: DateFilter,
    active: &DateFilter,
    left_pad: u16,
) -> Element<'a, Message> {
    let is_active = &filter == active;
    let btn = button(text(label).size(13))
        .on_press(Message::SetDateFilter(filter))
        .padding(Padding::from([2, 6]))
        .width(Length::Fill);

    let btn = if is_active {
        btn.style(button::primary)
    } else {
        btn.style(button::text)
    };

    if left_pad > 0 {
        row![Space::new().width(left_pad as f32), btn]
            .spacing(0)
            .into()
    } else {
        btn.into()
    }
}

fn arrow_button<'a>(label: &str, key: DateExpansionKey) -> Element<'a, Message> {
    button(text(label.to_owned()).size(12))
        .on_press(Message::ToggleDateExpansion(key))
        .padding(Padding::from([2, 4]))
        .style(button::text)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_photo(id: i64, date: Option<&str>) -> Photo {
        Photo {
            id,
            file_path: format!("/photos/test_{id}.jpg"),
            file_hash: format!("hash{id}"),
            file_size: 1000,
            width: Some(100),
            height: Some(100),
            camera_make: None,
            camera_model: None,
            lens: None,
            focal_length: None,
            aperture: None,
            shutter_speed: None,
            iso: None,
            date_taken: date.map(String::from),
            imported_at: "2026-01-01".into(),
            thumbnail_path: None,
        }
    }

    #[test]
    fn parse_date_valid() {
        assert_eq!(parse_date(Some("2026-02-05 14:30:00")), Some((2026, 2, 5)));
        assert_eq!(parse_date(Some("2025-12-31")), Some((2025, 12, 31)));
    }

    #[test]
    fn parse_date_none_and_short() {
        assert_eq!(parse_date(None), None);
        assert_eq!(parse_date(Some("")), None);
        assert_eq!(parse_date(Some("2026")), None);
        assert_eq!(parse_date(Some("2026-02")), None);
    }

    #[test]
    fn parse_date_garbage() {
        assert_eq!(parse_date(Some("not-a-date!!")), None);
        assert_eq!(parse_date(Some("abcd-ef-gh")), None);
    }

    #[test]
    fn parse_date_invalid_range() {
        assert_eq!(parse_date(Some("2026-13-01")), None);
        assert_eq!(parse_date(Some("2026-00-01")), None);
        assert_eq!(parse_date(Some("2026-01-00")), None);
        assert_eq!(parse_date(Some("2026-01-32")), None);
    }

    #[test]
    fn build_tree_grouping() {
        let photos = vec![
            make_photo(1, Some("2026-02-05 10:00:00")),
            make_photo(2, Some("2026-02-05 11:00:00")),
            make_photo(3, Some("2026-02-03 09:00:00")),
            make_photo(4, Some("2026-01-15 08:00:00")),
            make_photo(5, Some("2025-06-01 12:00:00")),
            make_photo(6, None),
        ];

        let tree = build_date_tree(&photos);
        assert_eq!(tree.total, 6);
        assert_eq!(tree.unknown_count, 1);
        assert_eq!(tree.years.len(), 2);

        // Newest first
        assert_eq!(tree.years[0].year, 2026);
        assert_eq!(tree.years[0].count, 4);
        assert_eq!(tree.years[1].year, 2025);
        assert_eq!(tree.years[1].count, 1);

        // 2026 months
        let months = &tree.years[0].months;
        assert_eq!(months.len(), 2);
        assert_eq!(months[0].month, 1);
        assert_eq!(months[0].count, 1);
        assert_eq!(months[1].month, 2);
        assert_eq!(months[1].count, 3);

        // Feb days
        let days = &months[1].days;
        assert_eq!(days.len(), 2);
        assert_eq!(days[0].day, 3);
        assert_eq!(days[0].count, 1);
        assert_eq!(days[1].day, 5);
        assert_eq!(days[1].count, 2);
    }

    #[test]
    fn filter_matches_all() {
        let photo = make_photo(1, Some("2026-02-05 10:00:00"));
        assert!(DateFilter::All.matches(&photo));
    }

    #[test]
    fn filter_matches_year() {
        let photo = make_photo(1, Some("2026-02-05 10:00:00"));
        assert!(DateFilter::Year(2026).matches(&photo));
        assert!(!DateFilter::Year(2025).matches(&photo));
    }

    #[test]
    fn filter_matches_month() {
        let photo = make_photo(1, Some("2026-02-05 10:00:00"));
        assert!(DateFilter::Month(2026, 2).matches(&photo));
        assert!(!DateFilter::Month(2026, 1).matches(&photo));
    }

    #[test]
    fn filter_matches_day() {
        let photo = make_photo(1, Some("2026-02-05 10:00:00"));
        assert!(DateFilter::Day(2026, 2, 5).matches(&photo));
        assert!(!DateFilter::Day(2026, 2, 4).matches(&photo));
    }

    #[test]
    fn filter_matches_unknown() {
        let with_date = make_photo(1, Some("2026-02-05 10:00:00"));
        let without = make_photo(2, None);
        assert!(!DateFilter::Unknown.matches(&with_date));
        assert!(DateFilter::Unknown.matches(&without));
    }
}
