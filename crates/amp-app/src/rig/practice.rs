use super::Rig;
use crate::knob::accent;
use amp_preset::PracticeSheet;
use gpui_kit::{
    component::{
        ActiveTheme, Sizable, StyledExt,
        button::{Button, ButtonVariants},
        h_flex, v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use std::fs;

/// What the Practice view is showing.
#[derive(Default)]
pub(super) struct Practice {
    pub(super) sheets: Vec<PracticeSheet>,
    selected: usize,
    /// The section whose tone was loaded last.
    active: Option<usize>,
}

impl Rig {
    /// Reloads sheets from the library, adding the warm-up to an empty one.
    pub(super) fn refresh_sheets(&mut self) {
        let result = self.library.practice_sheets().and_then(|(sheets, errors)| {
            if !sheets.is_empty() || !errors.is_empty() {
                return Ok((sheets, errors));
            }
            let warmup = PracticeSheet::warmup();
            self.library.save_sheet(&warmup)?;
            Ok((vec![warmup], errors))
        });
        match result {
            Ok((sheets, errors)) => {
                self.practice.sheets = sheets;
                self.practice.selected = self
                    .practice
                    .selected
                    .min(self.practice.sheets.len().saturating_sub(1));
                if let Some(error) = errors.first() {
                    self.error = Some(error.to_string().into());
                }
            }
            Err(error) => self.error = Some(error.to_string().into()),
        }
    }

    fn select_sheet(&mut self, index: usize, cx: &mut Context<Self>) {
        self.practice.selected = index;
        self.practice.active = None;
        cx.notify();
    }

    fn import_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pick_path(false, "Import tab", window, cx, |this, path, _, cx| {
            let text = match fs::read_to_string(&path) {
                Ok(text) => text,
                Err(error) => return this.set_error(error, cx),
            };
            let title = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Imported tab".to_owned());
            let sheet = PracticeSheet::from_text(title.clone(), &text);
            if sheet.sections.is_empty() {
                return this.set_error(format!("{title} has no tab in it"), cx);
            }
            if let Err(error) = this.library.save_sheet(&sheet) {
                return this.set_error(error, cx);
            }
            this.refresh_sheets();
            if let Some(index) = this.practice.sheets.iter().position(|s| s.title == title) {
                this.select_sheet(index, cx);
            }
            cx.notify();
        });
    }

    fn load_section_tone(&mut self, section: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sheet) = self.practice.sheets.get(self.practice.selected) else {
            return;
        };
        let Some(name) = sheet.sections[section].tone.clone() else {
            return;
        };
        let preset = self
            .presets
            .iter()
            .find(|entry| entry.preset.name == name)
            .map(|entry| entry.preset.clone());
        match preset {
            Some(preset) => {
                self.practice.active = Some(section);
                self.load_preset(preset, window, cx);
            }
            None => self.set_error(format!("The tone \"{name}\" is not in this library"), cx),
        }
        cx.notify();
    }

    /// Links the tone on screen. It has to be a saved one, because a link is a
    /// name, and an unsaved tone has nothing for the name to find.
    fn link_section_tone(&mut self, section: usize, cx: &mut Context<Self>) {
        let name = self.preset.name.clone();
        if !self.presets.iter().any(|entry| entry.preset.name == name) {
            return self.set_error("Save this tone first, then link it", cx);
        }
        let Some(sheet) = self.practice.sheets.get_mut(self.practice.selected) else {
            return;
        };
        sheet.sections[section].tone = Some(name);
        if let Err(error) = self.library.save_sheet(sheet) {
            self.set_error(error, cx);
        }
        self.practice.active = Some(section);
        cx.notify();
    }

    fn render_section(
        &self,
        index: usize,
        sheet: &PracticeSheet,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let section = &sheet.sections[index];
        let active = self.practice.active == Some(index);
        let font = cx.theme().mono_font_family.clone();
        v_flex()
            .gap_2()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(if active { accent() } else { cx.theme().border })
            .bg(cx.theme().secondary)
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(div().font_semibold().child(section.name.clone()))
                    .child(
                        h_flex()
                            .gap_1()
                            .when_some(section.tone.clone(), |row, tone| {
                                row.child(
                                    Button::new(("section-tone", index))
                                        .small()
                                        .when(active, |button| button.primary())
                                        .when(!active, |button| button.outline())
                                        .label(format!("Load {tone}"))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.load_section_tone(index, window, cx);
                                        })),
                                )
                            })
                            .child(
                                Button::new(("section-link", index))
                                    .small()
                                    .ghost()
                                    .label(if section.tone.is_some() {
                                        "Relink to current tone"
                                    } else {
                                        "Link current tone"
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.link_section_tone(index, cx);
                                    })),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .id(("section-tab", index))
                    .overflow_x_scroll()
                    .font_family(font)
                    .text_sm()
                    .children(section.tab.lines().map(|line| {
                        // An empty div has no height, and tab needs its gaps.
                        let line = if line.is_empty() { " " } else { line };
                        div().whitespace_nowrap().child(line.to_owned())
                    })),
            )
    }

    pub(super) fn render_practice_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let sheet = self.practice.sheets.get(self.practice.selected).cloned();
        let selected = self.practice.selected;
        let sheet_buttons = self
            .practice
            .sheets
            .iter()
            .enumerate()
            .map(|(index, sheet)| {
                Button::new(("sheet", index))
                    .small()
                    .label(sheet.title.clone())
                    .when(index == selected, |button| button.primary())
                    .when(index != selected, |button| button.outline())
                    .on_click(cx.listener(move |this, _, _, cx| this.select_sheet(index, cx)))
            });
        let sheet_buttons: Vec<_> = sheet_buttons.collect();
        let sections: Vec<AnyElement> = sheet.as_ref().map_or(Vec::new(), |sheet| {
            (0..sheet.sections.len())
                .map(|index| self.render_section(index, sheet, cx).into_any_element())
                .collect()
        });
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1()
                    .children(sheet_buttons)
                    .child(
                        Button::new("import-tab")
                            .small()
                            .ghost()
                            .label("Import tab")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.import_tab(window, cx);
                            })),
                    ),
            )
            .children(sections)
    }
}
