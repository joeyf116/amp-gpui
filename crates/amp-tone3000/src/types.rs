use serde::{Deserialize, Deserializer};

/// The API sends `null` for some empty lists.
fn null_as_empty<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::deserialize(deserializer)?.unwrap_or_default())
}

/// One page of a list, as the API sends it.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Page<T> {
    pub data: Vec<T>,
    #[serde(default)]
    pub page: u32,
    #[serde(default)]
    pub total_pages: u32,
}

impl<T> Page<T> {
    pub fn has_next(&self) -> bool {
        self.page < self.total_pages
    }
}

/// The signed-in account.
#[derive(Clone, Debug, Deserialize)]
pub struct User {
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
}

/// Who made a tone. Terms require keeping this with the tone wherever it shows.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Creator {
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
}

/// A make or tag on a tone.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Label {
    #[serde(default)]
    pub name: String,
}

/// A capture project: one piece of gear, holding one or more models.
#[derive(Clone, Debug, Deserialize)]
pub struct Tone {
    pub id: u64,
    #[serde(default)]
    pub user: Option<Creator>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub gear: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
    /// Only sent when a single tone is fetched.
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default, deserialize_with = "null_as_empty")]
    pub images: Vec<String>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub a1_models_count: u64,
    #[serde(default)]
    pub a2_models_count: u64,
    #[serde(default, deserialize_with = "null_as_empty")]
    pub makes: Vec<Label>,
    #[serde(default, deserialize_with = "null_as_empty")]
    pub tags: Vec<Label>,
}

impl Tone {
    pub fn image(&self) -> Option<&str> {
        self.images.first().map(String::as_str)
    }

    pub fn creator(&self) -> Creator {
        self.user.clone().unwrap_or_default()
    }

    pub fn is_ir(&self) -> bool {
        self.format.as_deref() == Some("ir")
    }

    /// Whether the app can run this tone's models.
    pub fn can_load(&self) -> bool {
        matches!(self.format.as_deref(), Some("nam" | "ir"))
    }

    pub fn gear_label(&self) -> &str {
        match self.gear.as_deref() {
            Some("amp") => "Amp",
            Some("amp-cab" | "full-rig") => "Amp + Cab",
            Some("pedal") => "Pedal",
            Some("outboard") => "Outboard",
            Some("cab") => "Cab",
            Some("space") => "Space",
            Some("experimental") => "Experimental",
            Some("ir") => "IR",
            Some(other) => other,
            None => "Gear",
        }
    }

    pub fn format_label(&self) -> &str {
        match self.format.as_deref() {
            Some("nam") => "NAM",
            Some("ir") => "IR",
            Some("aida-x") => "AIDA-X",
            Some("aa-snapshot") => "AA Snapshot",
            Some("proteus") => "Proteus",
            Some(other) => other,
            None => "Unknown format",
        }
    }

    pub fn license_label(&self) -> Option<&str> {
        Some(match self.license.as_deref()? {
            "t3k" => "TONE3000 license",
            "cc-by" => "CC BY",
            "cc-by-sa" => "CC BY-SA",
            "cc-by-nc" => "CC BY-NC",
            "cc-by-nc-sa" => "CC BY-NC-SA",
            "cc-by-nd" => "CC BY-ND",
            "cc-by-nc-nd" => "CC BY-NC-ND",
            "cco" => "CC0",
            other => other,
        })
    }
}

/// One downloadable file in a tone.
#[derive(Clone, Debug, Deserialize)]
pub struct Model {
    pub id: u64,
    #[serde(default)]
    pub tone_id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub model_url: String,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub architecture_version: Option<String>,
}

impl Model {
    /// Size and architecture, for telling a tone's models apart.
    pub fn detail(&self) -> String {
        let architecture = self
            .architecture_version
            .as_deref()
            .map(|version| format!("A{version}"));
        [architecture, self.size.clone()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" / ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tone_survives_nulls_and_missing_fields() {
        let tone: Tone = serde_json::from_str(r#"{"id":7,"images":null,"extra":1}"#).unwrap();
        assert_eq!(tone.id, 7);
        assert!(tone.image().is_none());
        assert_eq!(tone.creator().username, "");
        assert!(!tone.can_load());
    }

    #[test]
    fn labels_read_the_way_the_site_shows_them() {
        let tone: Tone = serde_json::from_str(
            r#"{"id":1,"gear":"amp-cab","format":"nam","license":"cc-by-nc"}"#,
        )
        .unwrap();
        assert_eq!(tone.gear_label(), "Amp + Cab");
        assert_eq!(tone.format_label(), "NAM");
        assert_eq!(tone.license_label(), Some("CC BY-NC"));
        assert!(tone.can_load());
    }

    #[test]
    fn a_model_describes_its_size_and_architecture() {
        let model: Model =
            serde_json::from_str(r#"{"id":1,"architecture_version":"2","size":"standard"}"#)
                .unwrap();
        assert_eq!(model.detail(), "A2 / standard");
    }
}
