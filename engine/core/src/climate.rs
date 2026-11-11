//! Climate data for visual explanations: reading it from text, checking it and summarising it.
//!
//! Nothing here invents a number. Values are read from text exactly as written or supplied by the
//! learner, and everything derived from them is plain arithmetic.

use std::sync::LazyLock;

use fancy_regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::{require_book, round1, Error, Origin, Result};

/// Sixteen compass points, clockwise from north; the bearing of each is its index times 22.5 degrees.
pub const DIRECTIONS: [&str; 16] = ["N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW", "NNW"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const MONTH_NAMES: [&str; 12] = [
    "january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december",
];
const MIN_MONTHS_IN_TABLE: usize = 6;
const MIN_WINDS_IN_TABLE: usize = 3;
const MAX_NAME_CHARS: usize = 120;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MonthTemperature {
    /// 1 for January to 12 for December.
    pub month: u32,
    pub max: f64,
    pub min: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Wind {
    pub direction: String,
    /// Percentage of the time the wind blows from here.
    pub share: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Tables {
    #[serde(default)]
    pub months: Vec<MonthTemperature>,
    #[serde(default)]
    pub winds: Vec<Wind>,
}

fn direction_index(direction: &str) -> Option<usize> {
    DIRECTIONS.iter().position(|known| *known == direction)
}

impl Tables {
    /// Checks the data and puts it in calendar and compass order.
    pub fn validated(mut self) -> Result<Self> {
        let invalid = |reason: String| Err(Error::Invalid(reason));
        if self.months.is_empty() && self.winds.is_empty() {
            return invalid("Enter at least one month of temperatures or one wind direction.".into());
        }
        for item in &self.months {
            if !(1..=12).contains(&item.month) {
                return invalid("A month must be between 1 and 12.".into());
            }
            let plausible = |value: f64| value.is_finite() && (-90.0..=60.0).contains(&value);
            if !plausible(item.max) || !plausible(item.min) {
                return invalid(format!("{}: temperatures must be between -90 and 60 °C.", MONTHS[item.month as usize - 1]));
            }
            if item.min > item.max {
                return invalid(format!("{}: the minimum is above the maximum.", MONTHS[item.month as usize - 1]));
            }
        }
        for item in &self.winds {
            if direction_index(&item.direction).is_none() {
                return invalid(format!("Unknown wind direction: {}", item.direction));
            }
            if !item.share.is_finite() || !(0.0..=100.0).contains(&item.share) {
                return invalid(format!("{}: a wind percentage must be between 0 and 100.", item.direction));
            }
        }
        self.months.sort_by_key(|item| item.month);
        if self.months.windows(2).any(|pair| pair[0].month == pair[1].month) {
            return invalid("A month appears more than once.".into());
        }
        self.winds.sort_by_key(|item| direction_index(&item.direction));
        if self.winds.windows(2).any(|pair| pair[0].direction == pair[1].direction) {
            return invalid("A wind direction appears more than once.".into());
        }
        // The remainder below 100 is calm; a little above allows for rounding in the source.
        if self.winds.iter().map(|item| item.share).sum::<f64>() > 100.5 {
            return invalid("The wind percentages add up to more than 100.".into());
        }
        Ok(self)
    }
}

// --- Reading data from text ---

const NUMBER: &str = r"[-−]?\d+(?:\.\d+)?";
const DIRECTION: &str = r"(?P<dir>north[\s-]?east|north[\s-]?west|south[\s-]?east|south[\s-]?west|north|south|east|west|NNE|ENE|ESE|SSE|SSW|WSW|WNW|NNW|NE|SE|SW|NW|N|E|S|W)";

static MONTH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)\b(?P<name>{names}|jan|feb|mar|apr|jun|jul|aug|sept?|oct|nov|dec)\b\.?[\s:,|;]+(?P<a>{NUMBER})\s*(?:°\s*C?)?(?:\s*[/,|;]\s*|\s+(?:[–—-]\s+|to\s+)?)(?P<b>{NUMBER})",
        names = MONTH_NAMES.join("|"),
    ))
    .expect("month pattern")
});
// A direction counts as wind data when a percent sign follows the number, or the pair fills a line.
static WIND_PERCENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"(?i)(?<![A-Za-z]){DIRECTION}(?![A-Za-z])[\s:,|;]+(?P<share>\d+(?:\.\d+)?)\s*%")).expect("wind pattern")
});
static WIND_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"(?im)^\s*{DIRECTION}(?![A-Za-z])[\s:,|;]+(?P<share>\d+(?:\.\d+)?)\s*$")).expect("wind line pattern")
});

fn number(text: &str) -> Option<f64> {
    text.replace('−', "-").parse().ok()
}

fn direction_code(text: &str) -> String {
    let mut compact: String = text.to_lowercase().chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    for (word, letter) in [("north", "n"), ("east", "e"), ("south", "s"), ("west", "w")] {
        compact = compact.replace(word, letter);
    }
    compact.to_uppercase()
}

/// Reads monthly temperature pairs and wind percentages from a pasted or extracted table.
///
/// A month needs two numbers after it; the larger is taken as the maximum. Anything that does not
/// fit these shapes is ignored rather than guessed at, and the first value found for a month or
/// direction wins.
pub fn parse_text(text: &str) -> Tables {
    let mut months: Vec<MonthTemperature> = Vec::new();
    for found in MONTH.captures_iter(text).flatten() {
        let (Some(a), Some(b)) = (number(&found["a"]), number(&found["b"])) else { continue };
        if a.min(b) < -90.0 || a.max(b) > 60.0 {
            continue;
        }
        let prefix: String = found["name"].to_lowercase().chars().take(3).collect();
        let month = MONTH_NAMES.iter().position(|name| name.starts_with(&prefix)).expect("a matched month name") as u32 + 1;
        if !months.iter().any(|item| item.month == month) {
            months.push(MonthTemperature { month, max: a.max(b), min: a.min(b) });
        }
    }
    months.sort_by_key(|item| item.month);

    let mut winds: Vec<Wind> = Vec::new();
    for pattern in [&*WIND_PERCENT, &*WIND_LINE] {
        for found in pattern.captures_iter(text).flatten() {
            let Some(share) = number(&found["share"]) else { continue };
            let direction = direction_code(&found["dir"]);
            if share <= 100.0 && !winds.iter().any(|item| item.direction == direction) {
                winds.push(Wind { direction, share });
            }
        }
    }
    if winds.iter().map(|item| item.share).sum::<f64>() > 100.5 {
        winds.clear(); // Not a frequency table after all.
    }
    winds.sort_by_key(|item| direction_index(&item.direction));
    Tables { months, winds }
}

/// A passage of a book that holds a climate table, with what was read from it.
#[derive(Clone, Debug, Serialize)]
pub struct Candidate {
    #[serde(flatten)]
    pub origin: Origin,
    #[serde(flatten)]
    pub tables: Tables,
}

pub fn find_in_book(conn: &Connection, book_id: i64) -> Result<Vec<Candidate>> {
    require_book(conn, book_id)?;
    let mut statement = conn.prepare(
        "SELECT p.id, p.section_id, s.title, p.start_page, p.end_page, p.text
         FROM passages p JOIN sections s ON s.id = p.section_id
         WHERE p.book_id = ? ORDER BY p.start_page, p.ordinal",
    )?;
    let rows = statement.query_map([book_id], |row| {
        let origin = Origin { passage_id: row.get(0)?, section_id: row.get(1)?, section_title: row.get(2)?, start_page: row.get(3)?, end_page: row.get(4)? };
        Ok((origin, row.get::<_, String>(5)?))
    })?;
    let mut found = Vec::new();
    for row in rows {
        let (origin, text) = row?;
        let mut tables = parse_text(&text);
        if tables.months.len() < MIN_MONTHS_IN_TABLE {
            tables.months.clear();
        }
        if tables.winds.len() < MIN_WINDS_IN_TABLE {
            tables.winds.clear();
        }
        if !tables.months.is_empty() || !tables.winds.is_empty() {
            found.push(Candidate { origin, tables });
            if found.len() == 20 {
                break;
            }
        }
    }
    Ok(found)
}

// --- What the data says, and which drawings it supports ---

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MonthMax {
    pub month: u32,
    pub max: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MonthMin {
    pub month: u32,
    pub min: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Summary {
    pub prevailing_wind: Option<Wind>,
    pub calm_share: Option<f64>,
    pub hottest: Option<MonthMax>,
    pub coolest: Option<MonthMin>,
    pub mean_daily_range: Option<f64>,
}

pub fn summarise(data: &Tables) -> Summary {
    let mut summary = Summary::default();
    // On a tie the first in order wins, as it did before the move from Python.
    if let Some(top) = data.winds.iter().reduce(|best, item| if item.share > best.share { item } else { best }) {
        summary.prevailing_wind = Some(top.clone());
        summary.calm_share = Some(round1((100.0 - data.winds.iter().map(|item| item.share).sum::<f64>()).max(0.0)));
    }
    if !data.months.is_empty() {
        let hottest = data.months.iter().reduce(|best, item| if item.max > best.max { item } else { best }).expect("months");
        let coolest = data.months.iter().reduce(|best, item| if item.min < best.min { item } else { best }).expect("months");
        summary.hottest = Some(MonthMax { month: hottest.month, max: hottest.max });
        summary.coolest = Some(MonthMin { month: coolest.month, min: coolest.min });
        summary.mean_daily_range = Some(round1(data.months.iter().map(|item| item.max - item.min).sum::<f64>() / data.months.len() as f64));
    }
    summary
}

#[derive(Clone, Debug, Serialize)]
pub struct FigureOffer {
    pub kind: &'static str,
    pub title: &'static str,
    pub available: bool,
    /// What data the figure needs, shown when it is unavailable.
    pub needs: &'static str,
    /// What in the figure is data and what is schematic.
    pub basis: &'static str,
}

/// Chooses the drawings this data can support, and says what an unavailable drawing needs.
pub fn figures(data: &Tables) -> Vec<FigureOffer> {
    const WIND: &str = "Needs wind directions with the percentage of time for each.";
    const TEMPERATURE: &str = "Needs monthly maximum and minimum temperatures.";
    let (winds, months) = (!data.winds.is_empty(), !data.months.is_empty());
    vec![
        FigureOffer { kind: "wind_rose", title: "Wind rose", available: winds, needs: WIND, basis: "Drawn exactly from the wind percentages." },
        FigureOffer {
            kind: "comfort_chart", title: "Temperature and comfort", available: months, needs: TEMPERATURE,
            basis: "Bars are drawn exactly from the temperatures. The comfort band is an assumption you can change.",
        },
        FigureOffer {
            kind: "section", title: "Air through a room", available: winds, needs: WIND,
            basis: "A schematic section, not to scale. The air path illustrates the principle and is not a simulation.",
        },
        FigureOffer {
            kind: "model_3d", title: "3D model", available: winds, needs: WIND,
            basis: "A schematic model, not to scale. Wind directions and percentages are from the data. The air path illustrates the principle and is not a simulation.",
        },
    ]
}

// --- Saved data ---

/// What the learner sends to save: the tables, a name, and where the values came from.
#[derive(Clone, Debug, Deserialize)]
pub struct Draft {
    pub name: String,
    #[serde(default = "entered")]
    pub source: String,
    #[serde(default)]
    pub passage_id: Option<i64>,
    #[serde(flatten)]
    pub tables: Tables,
}

fn entered() -> String {
    "entered".into()
}

#[derive(Clone, Debug, Serialize)]
pub struct Dataset {
    pub id: i64,
    pub book_id: i64,
    pub name: String,
    pub source: String,
    pub origin: Option<Origin>,
    #[serde(flatten)]
    pub tables: Tables,
    pub summary: Summary,
    pub figures: Vec<FigureOffer>,
    pub created_at: String,
}

const SELECT: &str = "
    SELECT d.id, d.book_id, d.name, d.source, d.data, d.created_at, d.passage_id, p.section_id, s.title, p.start_page, p.end_page
    FROM climate_datasets d
    LEFT JOIN passages p ON p.id = d.passage_id
    LEFT JOIN sections s ON s.id = p.section_id";

fn to_dataset(row: &rusqlite::Row<'_>) -> rusqlite::Result<Dataset> {
    let data: String = row.get(4)?;
    let tables: Tables = serde_json::from_str(&data).unwrap_or_default();
    let origin = match (row.get::<_, Option<i64>>(6)?, row.get::<_, Option<i64>>(7)?) {
        (Some(passage_id), Some(section_id)) => {
            Some(Origin { passage_id, section_id, section_title: row.get(8)?, start_page: row.get(9)?, end_page: row.get(10)? })
        }
        _ => None,
    };
    Ok(Dataset {
        id: row.get(0)?,
        book_id: row.get(1)?,
        name: row.get(2)?,
        source: row.get(3)?,
        origin,
        summary: summarise(&tables),
        figures: figures(&tables),
        tables,
        created_at: row.get(5)?,
    })
}

fn fetch(conn: &Connection, id: i64) -> Result<Dataset> {
    conn.query_row(&format!("{SELECT} WHERE d.id = ?"), [id], to_dataset).optional()?.ok_or(Error::NotFound("Climate data"))
}

pub fn list(conn: &Connection, book_id: i64) -> Result<Vec<Dataset>> {
    require_book(conn, book_id)?;
    let mut statement = conn.prepare(&format!("{SELECT} WHERE d.book_id = ? ORDER BY d.created_at DESC, d.id DESC"))?;
    let rows = statement.query_map([book_id], to_dataset)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Creates the data, or replaces it when an id is given.
pub fn save(conn: &Connection, book_id: i64, draft: Draft, id: Option<i64>) -> Result<Dataset> {
    let book_id = match id {
        Some(id) => fetch(conn, id)?.book_id,
        None => {
            require_book(conn, book_id)?;
            book_id
        }
    };
    let name = draft.name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(Error::Invalid("Give the data a name of up to 120 characters.".into()));
    }
    let passage_id = match draft.source.as_str() {
        "entered" => None,
        "book" => {
            let in_book: Option<i64> = conn
                .query_row("SELECT id FROM passages WHERE id = ? AND book_id = ?", params![draft.passage_id, book_id], |row| row.get(0))
                .optional()?;
            Some(in_book.ok_or_else(|| Error::Invalid("Data marked as from the book needs a passage of that book.".into()))?)
        }
        _ => return Err(Error::Invalid("The source must be the book or the learner.".into())),
    };
    let data = serde_json::to_string(&draft.tables.validated()?).expect("tables serialise");
    let saved = match id {
        Some(id) => {
            conn.execute(
                "UPDATE climate_datasets SET name = ?, source = ?, passage_id = ?, data = ? WHERE id = ?",
                params![name, draft.source, passage_id, data, id],
            )?;
            id
        }
        None => {
            conn.execute(
                "INSERT INTO climate_datasets (book_id, name, source, passage_id, data) VALUES (?, ?, ?, ?, ?)",
                params![book_id, name, draft.source, passage_id, data],
            )?;
            conn.last_insert_rowid()
        }
    };
    fetch(conn, saved)
}

pub fn delete(conn: &Connection, id: i64) -> Result<()> {
    match conn.execute("DELETE FROM climate_datasets WHERE id = ?", [id])? {
        0 => Err(Error::NotFound("Climate data")),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{book, library, section_id};

    const TEMPERATURES: [(f64, f64); 12] = [
        (32.0, 23.0), (33.0, 25.0), (33.0, 25.0), (32.0, 25.0), (31.0, 24.0), (29.0, 23.0),
        (28.0, 23.0), (28.0, 23.0), (29.0, 23.0), (30.0, 23.0), (31.0, 24.0), (32.0, 23.0),
    ];

    fn month(month: u32, max: f64, min: f64) -> MonthTemperature {
        MonthTemperature { month, max, min }
    }

    fn wind(direction: &str, share: f64) -> Wind {
        Wind { direction: direction.into(), share }
    }

    fn draft(months: Vec<MonthTemperature>, winds: Vec<Wind>) -> Draft {
        Draft { name: "Site A".into(), source: "entered".into(), passage_id: None, tables: Tables { months, winds } }
    }

    fn site_a() -> Draft {
        draft(
            vec![month(2, 33.0, 25.0), month(1, 32.0, 23.0), month(7, 28.0, 23.0)],
            vec![wind("S", 22.0), wind("SW", 34.0), wind("W", 15.0)],
        )
    }

    /// A book whose second section holds a temperature table and a wind table, as extraction leaves them.
    fn climate_book(conn: &Connection) -> i64 {
        let mut table = String::from("Mean monthly temperature in degrees Celsius, maximum then minimum.");
        for (name, (high, low)) in MONTHS.iter().zip(TEMPERATURES) {
            table.push_str(&format!(" {name} {high} {low}"));
        }
        table.push_str(" Wind frequency by direction. SW 34% S 22% W 15% SE 8%");
        book(conn, "Climate and Buildings", &[
            ("Why climate matters", &["A building should be shaped by the climate it stands in."]),
            ("Site records", &[&table]),
        ])
    }

    // --- Reading tables from text ---

    #[test]
    fn parse_reads_months_and_winds_in_common_layouts() {
        let parsed = parse_text("Month Max Min\nJanuary 32.5 23\nFeb: 33 / 25\nMar 25 – 33\nSept. 29°C 23\nWind\nSW 34%\nsouth-west 99%\nNorth East 4 %\nS 22\n");
        assert_eq!(parsed.months, [month(1, 32.5, 23.0), month(2, 33.0, 25.0), month(3, 33.0, 25.0), month(9, 29.0, 23.0)]);
        // Compass order; the first value for a direction wins.
        assert_eq!(parsed.winds, [wind("NE", 4.0), wind("S", 22.0), wind("SW", 34.0)]);
    }

    #[test]
    fn parse_reads_a_table_flattened_onto_one_line() {
        let parsed = parse_text("Jan 32 23 Feb 33 25 Mar 33 25. Winds: SW 34%, S 22%, W 15%.");
        assert_eq!(parsed.months.iter().map(|item| item.month).collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!(parsed.winds.iter().map(|item| item.direction.as_str()).collect::<Vec<_>>(), ["S", "SW", "W"]);
    }

    #[test]
    fn parse_ignores_what_is_not_climate_data() {
        for text in [
            "In May 1987 the survey began.",               // one number is not a temperature pair
            "The S 5 road runs north 12 km to the coast.", // no percent sign and not a table line
            "Jan 320 230",                                 // not plausible temperatures
            "N 60%\nS 70%",                                // more than 100 percent is not a frequency table
            "",
        ] {
            assert_eq!(parse_text(text), Tables::default(), "{text:?}");
        }
    }

    #[test]
    fn parse_handles_negative_temperatures() {
        assert_eq!(parse_text("Jan -3 -11\nFeb −1 −9").months, [month(1, -3.0, -11.0), month(2, -1.0, -9.0)]);
    }

    // --- Finding tables in a book ---

    #[test]
    fn tables_in_a_book_are_found_with_their_location() {
        let conn = library();
        let book = climate_book(&conn);
        let found = find_in_book(&conn, book).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].origin.section_title.as_str(), found[0].origin.start_page), ("Site records", 3));
        assert_eq!(found[0].tables.months.iter().map(|item| (item.max, item.min)).collect::<Vec<_>>(), TEMPERATURES);
        assert_eq!(found[0].tables.winds, [wind("SE", 8.0), wind("S", 22.0), wind("SW", 34.0), wind("W", 15.0)]);
        assert!(matches!(find_in_book(&conn, 999), Err(Error::NotFound("Book"))));
    }

    #[test]
    fn data_taken_from_the_book_keeps_its_source() {
        let conn = library();
        let book = climate_book(&conn);
        let table = find_in_book(&conn, book).unwrap().remove(0);
        let from_book = |passage_id| Draft { name: "Site records".into(), source: "book".into(), passage_id, tables: table.tables.clone() };
        let saved = save(&conn, book, from_book(Some(table.origin.passage_id)), None).unwrap();
        assert_eq!(saved.source, "book");
        assert_eq!(saved.origin.as_ref().map(|origin| origin.start_page), Some(3));
        assert!(matches!(save(&conn, book, from_book(Some(999_999)), None), Err(Error::Invalid(_))));
        assert!(matches!(save(&conn, book, from_book(None), None), Err(Error::Invalid(_))));
    }

    // --- Saving, summarising and choosing drawings ---

    #[test]
    fn saved_data_is_sorted_and_summarised() {
        let conn = library();
        let book = climate_book(&conn);
        save(&conn, book, site_a(), None).unwrap();
        let saved = list(&conn, book).unwrap().remove(0);
        assert_eq!((saved.source.as_str(), saved.origin.is_none()), ("entered", true));
        assert_eq!(saved.tables.months.iter().map(|item| item.month).collect::<Vec<_>>(), [1, 2, 7]);
        assert_eq!(saved.tables.winds.iter().map(|item| item.direction.as_str()).collect::<Vec<_>>(), ["S", "SW", "W"]);
        assert_eq!(saved.summary, Summary {
            prevailing_wind: Some(wind("SW", 34.0)),
            calm_share: Some(29.0),
            hottest: Some(MonthMax { month: 2, max: 33.0 }),
            coolest: Some(MonthMin { month: 1, min: 23.0 }),
            mean_daily_range: Some(7.3),
        });
    }

    #[test]
    fn saved_data_has_the_shape_the_window_expects() {
        let conn = library();
        let book = climate_book(&conn);
        let saved = serde_json::to_value(save(&conn, book, site_a(), None).unwrap()).unwrap();
        for key in ["id", "book_id", "name", "source", "origin", "months", "winds", "summary", "figures", "created_at"] {
            assert!(saved.get(key).is_some(), "missing {key}");
        }
        assert_eq!(saved["months"][0], serde_json::json!({"month": 1, "max": 32.0, "min": 23.0}));
        assert_eq!(saved["summary"]["prevailing_wind"], serde_json::json!({"direction": "SW", "share": 34.0}));
        let draft: Draft = serde_json::from_value(serde_json::json!({"name": "x", "months": [], "winds": [{"direction": "N", "share": 5}]})).unwrap();
        assert_eq!((draft.source.as_str(), draft.tables.winds.len()), ("entered", 1));
    }

    #[test]
    fn drawings_offered_depend_on_the_data() {
        let only_temperature = figures(&Tables { months: vec![month(1, 30.0, 20.0)], winds: vec![] });
        assert_eq!(only_temperature.iter().map(|figure| (figure.kind, figure.available)).collect::<Vec<_>>(), [
            ("wind_rose", false), ("comfort_chart", true), ("section", false), ("model_3d", false),
        ]);
        assert!(only_temperature.iter().all(|figure| !figure.basis.is_empty() && !figure.needs.is_empty()));
        assert!(figures(&site_a().tables).iter().all(|figure| figure.available));
    }

    #[test]
    fn invalid_data_is_rejected() {
        let conn = library();
        let book = climate_book(&conn);
        let cases = [
            draft(vec![], vec![]),
            draft(vec![month(1, 20.0, 25.0)], vec![]),
            draft(vec![month(13, 30.0, 20.0)], vec![]),
            draft(vec![month(1, 30.0, 20.0), month(1, 31.0, 21.0)], vec![]),
            draft(vec![month(1, 300.0, 20.0)], vec![]),
            draft(vec![], vec![wind("UP", 10.0)]),
            draft(vec![], vec![wind("N", 60.0), wind("S", 60.0)]),
            draft(vec![], vec![wind("N", 10.0), wind("N", 20.0)]),
            Draft { name: "  ".into(), ..site_a() },
            Draft { source: "guessed".into(), ..site_a() },
        ];
        for case in cases {
            assert!(matches!(save(&conn, book, case.clone(), None), Err(Error::Invalid(_))), "{case:?}");
        }
        assert!(list(&conn, book).unwrap().is_empty());
    }

    #[test]
    fn data_can_be_changed_and_removed_and_goes_with_its_book() {
        let conn = library();
        let book = climate_book(&conn);
        let saved = save(&conn, book, site_a(), None).unwrap();
        let change = Draft { name: "Site B".into(), ..draft(vec![], vec![wind("NE", 40.0)]) };
        let changed = save(&conn, 0, change, Some(saved.id)).unwrap();
        assert_eq!((changed.name.as_str(), changed.book_id), ("Site B", book));
        assert_eq!(changed.summary.prevailing_wind, Some(wind("NE", 40.0)));

        delete(&conn, saved.id).unwrap();
        assert!(matches!(delete(&conn, saved.id), Err(Error::NotFound(_))));
        assert!(matches!(save(&conn, book, site_a(), Some(saved.id)), Err(Error::NotFound(_))));

        save(&conn, book, site_a(), None).unwrap();
        conn.execute("DELETE FROM books WHERE id = ?", [book]).unwrap();
        assert!(matches!(list(&conn, book), Err(Error::NotFound("Book"))));
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM climate_datasets", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn a_passage_removed_from_the_book_leaves_the_data_without_an_origin() {
        let conn = library();
        let book = climate_book(&conn);
        let passage: i64 = conn.query_row("SELECT id FROM passages WHERE section_id = ?", [section_id(&conn, "Site records")], |row| row.get(0)).unwrap();
        let from_book = Draft { source: "book".into(), passage_id: Some(passage), ..site_a() };
        let saved = save(&conn, book, from_book, None).unwrap();
        conn.execute("DELETE FROM passages WHERE id = ?", [passage]).unwrap();
        assert!(list(&conn, book).unwrap().iter().any(|item| item.id == saved.id && item.origin.is_none()));
    }
}
