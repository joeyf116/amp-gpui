use crate::{
    Library, LibraryError, SCHEMA_VERSION,
    library::{PRACTICE, slug, write_json},
};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

/// Guitar tab to play along to, split into sections that can each call up a
/// tone. Sheets live in the library next to presets, so they sync with it.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PracticeSheet {
    pub schema: u32,
    pub title: String,
    pub sections: Vec<PracticeSection>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PracticeSection {
    pub name: String,
    /// The tone to load for this section, by preset name. Like a preset's own
    /// file name, this means a renamed tone needs linking again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<String>,
    /// Plain text tab. Spacing is meaningful, so it is never reflowed.
    pub tab: String,
}

impl PracticeSheet {
    /// Reads plain text tab. A line like `[Verse]` starts a new section, and
    /// anything before the first one goes in a section called Tab.
    pub fn from_text(title: impl Into<String>, text: &str) -> Self {
        let mut sections = Vec::new();
        let mut name = "Tab".to_owned();
        let mut lines: Vec<&str> = Vec::new();
        for line in text.trim_start_matches('\u{feff}').lines() {
            let trimmed = line.trim();
            let header = trimmed
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
                .filter(|header| !header.trim().is_empty());
            if let Some(header) = header {
                push_section(&mut sections, &name, &lines);
                name = header.trim().to_owned();
                lines.clear();
            } else {
                lines.push(line.trim_end());
            }
        }
        push_section(&mut sections, &name, &lines);
        Self {
            schema: SCHEMA_VERSION,
            title: title.into(),
            sections,
        }
    }

    /// An original warm-up, so the view has something to play on a new library.
    pub fn warmup() -> Self {
        Self::from_text("Daily warm-up", WARMUP)
    }
}

fn push_section(sections: &mut Vec<PracticeSection>, name: &str, lines: &[&str]) {
    let tab = lines.join("\n");
    let tab = tab.trim_matches('\n');
    if !tab.trim().is_empty() {
        sections.push(PracticeSection {
            name: name.to_owned(),
            tone: None,
            tab: tab.to_owned(),
        });
    }
}

const WARMUP: &str = "\
[Chromatic, one finger per fret]
e|-------------------------------------------1-2-3-4-|
B|---------------------------------1-2-3-4-----------|
G|-----------------------1-2-3-4---------------------|
D|-------------1-2-3-4-------------------------------|
A|---1-2-3-4-----------------------------------------|
E|-1-------------------------------------------------|

[A minor pentatonic, up and back]
e|-------------------------5-8-8-5-------------------|
B|---------------------5-8---------8-5---------------|
G|-----------------5-7-----------------7-5-----------|
D|-------------5-7-------------------------7-5-------|
A|---------5-7---------------------------------7-5---|
E|-----5-8-----------------------------------------8-|

[Palm-muted eighths for testing a gain tone]
e|-----------------------------------|
B|-----------------------------------|
G|-----------------------------------|
D|-----------------------------------|
A|-------------------------3-3-5-5---|
E|-0-0-0-0-0-0-0-0-0-0-0-0-----------|
  PM - - - - - - - - - - - - - - - -
";

impl Library {
    /// Every readable sheet, sorted by title, plus the errors for the rest.
    pub fn practice_sheets(&self) -> Result<(Vec<PracticeSheet>, Vec<LibraryError>), LibraryError> {
        let mut sheets = Vec::new();
        let mut errors = Vec::new();
        for file in fs::read_dir(self.root().join(PRACTICE))? {
            let path = file?.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                match read_sheet(path) {
                    Ok(sheet) => sheets.push(sheet),
                    Err(error) => errors.push(error),
                }
            }
        }
        sheets.sort_by_key(|sheet| sheet.title.to_lowercase());
        Ok((sheets, errors))
    }

    pub fn save_sheet(&self, sheet: &PracticeSheet) -> Result<(), LibraryError> {
        let slug = slug(&sheet.title).ok_or(LibraryError::InvalidName)?;
        let path = self.root().join(PRACTICE).join(format!("{slug}.json"));
        write_json(&path, sheet)
    }
}

fn read_sheet(path: PathBuf) -> Result<PracticeSheet, LibraryError> {
    let text = fs::read_to_string(&path)?;
    let sheet: PracticeSheet =
        serde_json::from_str(&text).map_err(|source| LibraryError::Json { path, source })?;
    if sheet.schema > SCHEMA_VERSION {
        return Err(LibraryError::NewerSchema(sheet.title));
    }
    Ok(sheet)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_tab_text_into_named_sections() {
        let text = "\u{feff}e|--0--|\r\n[Verse]\r\nE|--3--|\r\n\r\n[ Solo ]\nE|--5--|  \n";
        let sheet = PracticeSheet::from_text("Song", text);
        let names: Vec<_> = sheet.sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Tab", "Verse", "Solo"]);
        assert_eq!(sheet.sections[0].tab, "e|--0--|");
        assert_eq!(sheet.sections[1].tab, "E|--3--|");
        assert_eq!(sheet.sections[2].tab, "E|--5--|");
    }

    #[test]
    fn keeps_the_spacing_that_lines_tab_up() {
        let sheet = PracticeSheet::from_text("Song", "  PM - - -\nE|-0-0-0-|");
        assert_eq!(sheet.sections[0].tab, "  PM - - -\nE|-0-0-0-|");
    }

    #[test]
    fn drops_empty_sections() {
        let sheet = PracticeSheet::from_text("Song", "[Intro]\n\n[Outro]\nE|-1-|");
        assert_eq!(sheet.sections.len(), 1);
        assert_eq!(sheet.sections[0].name, "Outro");
    }

    #[test]
    fn empty_brackets_are_not_a_section_header() {
        let sheet = PracticeSheet::from_text("Song", "E|-1-|\n[]\nE|-2-|");
        assert_eq!(sheet.sections.len(), 1);
        assert_eq!(sheet.sections[0].tab, "E|-1-|\n[]\nE|-2-|");
    }

    #[test]
    fn the_warmup_has_a_section_for_each_exercise() {
        assert_eq!(PracticeSheet::warmup().sections.len(), 3);
    }

    #[test]
    fn sheets_round_trip_through_the_library_with_their_tones() {
        let directory = std::env::temp_dir().join(format!("amp-practice-{}", std::process::id()));
        let library = Library::open(&directory).unwrap();
        let mut sheet = PracticeSheet::warmup();
        sheet.sections[0].tone = Some("Crunch".into());
        library.save_sheet(&sheet).unwrap();
        fs::write(directory.join(PRACTICE).join("bad.json"), "{").unwrap();

        let (sheets, errors) = library.practice_sheets().unwrap();
        assert_eq!(sheets, [sheet]);
        assert_eq!(errors.len(), 1);
        fs::remove_dir_all(directory).unwrap();
    }
}
