//! The learner's settings, kept in the library database one key at a time.
//!
//! The Python backend still reads these rows (for the chosen model and the storage folder), so
//! each value is stored as JSON exactly as it stored them.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub llm_model: Option<String>,
    pub embedding_model: Option<String>,
    pub speech_model: Option<String>,
    pub voice: Option<String>,
    pub reading_speed: f64,
    pub explanation_depth: String,
    pub assessment_difficulty: String,
    pub compute_preference: String,
    pub document_storage_path: Option<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            llm_model: None,
            embedding_model: None,
            speech_model: None,
            voice: None,
            reading_speed: 1.0,
            explanation_depth: "standard".into(),
            assessment_difficulty: "standard".into(),
            compute_preference: "auto".into(),
            document_storage_path: None,
        }
    }
}

fn one_of(value: &str, name: &str, allowed: &[&str]) -> Result<()> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(Error::Invalid(format!("{name} must be one of: {}.", allowed.join(", "))))
    }
}

impl AppSettings {
    fn validated(self) -> Result<Self> {
        if !(0.5..=2.0).contains(&self.reading_speed) {
            return Err(Error::Invalid("Reading speed must be between 0.5 and 2.".into()));
        }
        one_of(&self.explanation_depth, "Explanation depth", &["brief", "standard", "deep"])?;
        one_of(&self.assessment_difficulty, "Assessment difficulty", &["gentle", "standard", "rigorous"])?;
        one_of(&self.compute_preference, "Compute preference", &["auto", "gpu", "cpu"])?;
        // The app works offline: a model that runs on someone else's computer is never an option.
        for model in [&self.llm_model, &self.embedding_model].into_iter().flatten() {
            if model.ends_with(":cloud") || model.ends_with("-cloud") {
                return Err(Error::Invalid("Cloud models cannot be selected.".into()));
            }
        }
        Ok(self)
    }
}

pub fn load(conn: &Connection) -> Result<AppSettings> {
    let mut statement = conn.prepare("SELECT key, value FROM app_settings")?;
    let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
    let mut stored = Map::new();
    for row in rows {
        let (key, value) = row?;
        // A value that is not JSON was not written by this app; the default stands in for it.
        if let Ok(value) = serde_json::from_str::<Value>(&value) {
            stored.insert(key, value);
        }
    }
    // Keys this version does not know are ignored, so an older version can open a newer library.
    serde_json::from_value(Value::Object(stored)).map_err(|error| Error::Invalid(format!("The saved settings could not be read: {error}")))
}

pub fn save(conn: &Connection, settings: AppSettings) -> Result<AppSettings> {
    let settings = settings.validated()?;
    let Value::Object(fields) = serde_json::to_value(&settings).expect("settings are plain data") else {
        unreachable!("settings serialise to an object")
    };
    let transaction = conn.unchecked_transaction()?;
    for (key, value) in fields {
        transaction.execute(
            "INSERT INTO app_settings (key, value) VALUES (?, ?) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value.to_string()],
        )?;
    }
    transaction.commit()?;
    load(conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::library;

    #[test]
    fn a_new_library_has_the_default_settings() {
        assert_eq!(load(&library()).unwrap(), AppSettings::default());
    }

    #[test]
    fn saved_settings_are_read_back() {
        let conn = library();
        let saved = save(&conn, AppSettings { reading_speed: 1.25, compute_preference: "cpu".into(), ..Default::default() }).unwrap();
        assert_eq!((saved.reading_speed, saved.compute_preference.as_str()), (1.25, "cpu"));
        assert_eq!(load(&conn).unwrap(), saved);
        // Stored the way the Python backend reads them.
        let raw: String = conn.query_row("SELECT value FROM app_settings WHERE key = 'compute_preference'", [], |row| row.get(0)).unwrap();
        assert_eq!(raw, "\"cpu\"");
    }

    #[test]
    fn values_out_of_range_are_refused_and_nothing_is_saved() {
        let conn = library();
        for bad in [
            AppSettings { reading_speed: 9.0, ..Default::default() },
            AppSettings { explanation_depth: "endless".into(), ..Default::default() },
            AppSettings { compute_preference: "quantum".into(), ..Default::default() },
        ] {
            assert!(matches!(save(&conn, bad), Err(Error::Invalid(_))));
        }
        assert_eq!(load(&conn).unwrap(), AppSettings::default());
    }

    #[test]
    fn cloud_models_are_refused() {
        let conn = library();
        for name in ["big:cloud", "big-cloud"] {
            let refused = save(&conn, AppSettings { llm_model: Some(name.into()), ..Default::default() });
            assert_eq!(refused.unwrap_err().to_string(), "Cloud models cannot be selected.");
            let refused = save(&conn, AppSettings { embedding_model: Some(name.into()), ..Default::default() });
            assert!(refused.is_err());
        }
        assert!(save(&conn, AppSettings { llm_model: Some("local:1b".into()), ..Default::default() }).is_ok());
    }

    #[test]
    fn rows_from_other_versions_do_not_break_reading() {
        let conn = library();
        conn.execute_batch(
            "INSERT INTO app_settings (key, value) VALUES ('from_the_future', '{\"a\": 1}');
             INSERT INTO app_settings (key, value) VALUES ('voice', 'not json');
             INSERT INTO app_settings (key, value) VALUES ('reading_speed', '1.5');",
        ).unwrap();
        let settings = load(&conn).unwrap();
        assert_eq!((settings.reading_speed, settings.voice), (1.5, None));
    }
}
