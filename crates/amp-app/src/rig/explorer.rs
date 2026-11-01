//! The Tone Explorer: every tone in the library, sorted onto shelves and
//! searchable. Tones load as they're clicked, so they can be compared by ear
//! with the guitar in hand.

use super::{Overlay, Rig};
use crate::style::{fog, hairline, moss, moss_glow, sheet};
use amp_preset::{PresetEntry, factory_tones};
use gpui_kit::{
    component::{
        Sizable, StyledExt,
        button::{Button, ButtonVariants},
        h_flex,
        input::{Input, InputState},
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use std::collections::BTreeSet;

/// How many tones the Recent shelf keeps.
const RECENT: usize = 12;

#[derive(Clone, Debug, PartialEq)]
enum Shelf {
    All,
    Favorites,
    Recent,
    Factory,
    Mine,
    Tag(String),
}

/// What the Explorer is showing.
pub(super) struct Explorer {
    pub(super) search: Entity<InputState>,
    shelf: Shelf,
    factory: Vec<String>,
}

impl Explorer {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Rig>) -> Self {
        Self {
            search: cx.new(|cx| InputState::new(window, cx).placeholder("Search tones and tags")),
            shelf: Shelf::All,
            factory: factory_tones().into_iter().map(|tone| tone.name).collect(),
        }
    }
}

/// The names a shelf is drawn from.
struct Lists<'a> {
    favorites: &'a [String],
    recent: &'a [String],
    factory: &'a [String],
}

fn on_shelf(entry: &PresetEntry, shelf: &Shelf, lists: &Lists) -> bool {
    let name = &entry.preset.name;
    match shelf {
        Shelf::All => true,
        Shelf::Favorites => lists.favorites.contains(name),
        Shelf::Recent => lists.recent.contains(name),
        Shelf::Factory => lists.factory.contains(name),
        Shelf::Mine => !lists.factory.contains(name),
        Shelf::Tag(tag) => entry.preset.tags.contains(tag),
    }
}

/// Every word of the search has to appear in the name, tags, or notes.
fn matches_search(entry: &PresetEntry, search: &str) -> bool {
    let preset = &entry.preset;
    let text = format!("{} {} {}", preset.name, preset.tags.join(" "), preset.notes).to_lowercase();
    search
        .to_lowercase()
        .split_whitespace()
        .all(|word| text.contains(word))
}

/// The tones to list, as indexes into `entries`. Recent keeps its newest-first
/// order, and everything else keeps the library's.
fn visible(entries: &[PresetEntry], shelf: &Shelf, search: &str, lists: &Lists) -> Vec<usize> {
    let mut found: Vec<usize> = (0..entries.len())
        .filter(|index| {
            on_shelf(&entries[*index], shelf, lists) && matches_search(&entries[*index], search)
        })
        .collect();
    if *shelf == Shelf::Recent {
        found.sort_by_key(|index| {
            lists
                .recent
                .iter()
                .position(|name| *name == entries[*index].preset.name)
        });
    }
    found
}

/// Moves `name` to the front of the recent list, which stays short.
fn remember_recent(recent: &mut Vec<String>, name: &str) {
    recent.retain(|other| other != name);
    recent.insert(0, name.to_owned());
    recent.truncate(RECENT);
}

impl Rig {
    pub(super) fn note_recent(&mut self, name: &str) {
        remember_recent(&mut self.settings.recent, name);
    }

    pub(super) fn is_favorite(&self, name: &str) -> bool {
        self.settings.favorites.iter().any(|tone| tone == name)
    }

    pub(super) fn toggle_favorite(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.is_favorite(name) {
            self.settings.favorites.retain(|tone| tone != name);
        } else {
            self.settings.favorites.push(name.to_owned());
        }
        self.save_settings(cx);
        cx.notify();
    }

    pub(super) fn toggle_tones(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay == Overlay::Tones {
            self.close_overlay(window, cx);
        } else {
            self.overlay = Overlay::Tones;
            cx.notify();
        }
    }

    fn lists(&self) -> Lists<'_> {
        Lists {
            favorites: &self.settings.favorites,
            recent: &self.settings.recent,
            factory: &self.explorer.factory,
        }
    }

    fn render_shelf(
        &self,
        id: usize,
        title: SharedString,
        shelf: Shelf,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let count = self
            .presets
            .iter()
            .filter(|entry| on_shelf(entry, &shelf, &self.lists()))
            .count();
        let current = self.explorer.shelf == shelf;
        h_flex()
            .id(("shelf", id))
            .justify_between()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded_md()
            .text_sm()
            .cursor_pointer()
            .when(current, |row| row.bg(moss_glow(0.14)).text_color(moss()))
            .when(!current, |row| row.hover(|row| row.bg(moss_glow(0.06))))
            .child(div().truncate().child(title))
            .child(div().text_xs().text_color(fog()).child(count.to_string()))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.explorer.shelf = shelf.clone();
                cx.notify();
            }))
    }

    fn render_tone_row(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let preset = self.presets[index].preset.clone();
        let current = preset.name == self.preset.name;
        let favorite = self.is_favorite(&preset.name);
        let star_name = preset.name.clone();
        let tags: Vec<_> = preset
            .tags
            .iter()
            .take(3)
            .map(|tag| {
                div()
                    .px_2()
                    .rounded_md()
                    .border_1()
                    .border_color(hairline())
                    .text_xs()
                    .text_color(fog())
                    .child(tag.clone())
            })
            .collect();
        h_flex()
            .id(("tone", index))
            .gap_3()
            .items_center()
            .px_3()
            .py_2()
            .rounded_md()
            .cursor_pointer()
            .when(current, |row| row.bg(moss_glow(0.12)))
            .when(!current, |row| row.hover(|row| row.bg(moss_glow(0.06))))
            .child(
                div()
                    .id(("star", index))
                    .w(px(18.))
                    .text_color(if favorite { moss() } else { fog() })
                    .child(if favorite { "\u{2605}" } else { "\u{2606}" })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_favorite(&star_name, cx);
                    })),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .font_semibold()
                            .when(current, |name| name.text_color(moss()))
                            .child(preset.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(fog())
                            .truncate()
                            .child(preset.notes.clone()),
                    ),
            )
            .child(h_flex().gap_1().flex_shrink_0().children(tags))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if event.click_count() >= 2 {
                    this.close_overlay(window, cx);
                } else {
                    this.load_preset(preset.clone(), window, cx);
                }
            }))
    }

    fn render_explorer_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_2()
            .items_center()
            .p_3()
            .border_t_1()
            .border_color(hairline())
            .child(
                div()
                    .w(px(260.))
                    .child(Input::new(&self.preset_name).small()),
            )
            .child(
                Button::new("save-as")
                    .small()
                    .primary()
                    .label("Save under this name")
                    .on_click(cx.listener(|this, _, _, cx| this.save_preset(cx))),
            )
            .child(div().flex_1())
            .child(
                Button::new("export")
                    .small()
                    .ghost()
                    .label("Export")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.export_pack(window, cx);
                    })),
            )
            .child(
                Button::new("import")
                    .small()
                    .ghost()
                    .label("Import")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.import_pack(window, cx);
                    })),
            )
            .child(
                Button::new("folder")
                    .small()
                    .ghost()
                    .label("Library folder")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.choose_library(window, cx);
                    })),
            )
    }

    pub(super) fn render_explorer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let search = self.explorer.search.read(cx).value().to_string();
        let tones = visible(&self.presets, &self.explorer.shelf, &search, &self.lists());
        let tags: BTreeSet<String> = self
            .presets
            .iter()
            .flat_map(|entry| entry.preset.tags.iter().cloned())
            .collect();
        let shelves: Vec<AnyElement> = [
            ("All tones", Shelf::All),
            ("Favorites", Shelf::Favorites),
            ("Recent", Shelf::Recent),
            ("Factory", Shelf::Factory),
            ("My tones", Shelf::Mine),
        ]
        .into_iter()
        .enumerate()
        .map(|(id, (title, shelf))| {
            self.render_shelf(id, title.into(), shelf, cx)
                .into_any_element()
        })
        .collect();
        let tag_shelves: Vec<AnyElement> = tags
            .into_iter()
            .enumerate()
            .map(|(id, tag)| {
                self.render_shelf(100 + id, tag.clone().into(), Shelf::Tag(tag), cx)
                    .into_any_element()
            })
            .collect();
        let rows: Vec<AnyElement> = tones
            .iter()
            .map(|index| self.render_tone_row(*index, cx).into_any_element())
            .collect();
        v_flex()
            .id("tone-explorer")
            .occlude()
            .absolute()
            .top(px(12.))
            .bottom(px(12.))
            .left(px(72.))
            .right(px(72.))
            .rounded_lg()
            .border_1()
            .border_color(hairline())
            .bg(sheet())
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .p_3()
                    .border_b_1()
                    .border_color(hairline())
                    .child(div().font_semibold().child("Tone Explorer"))
                    .child(div().flex_1().child(Input::new(&self.explorer.search)))
                    .child(
                        Button::new("close-explorer")
                            .small()
                            .ghost()
                            .label("Close")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_overlay(window, cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .child(
                        v_flex()
                            .id("shelves")
                            .w(px(190.))
                            .h_full()
                            .flex_shrink_0()
                            .overflow_y_scroll()
                            .p_2()
                            .gap_px()
                            .border_r_1()
                            .border_color(hairline())
                            .children(shelves)
                            .child(
                                div()
                                    .mt_3()
                                    .mb_1()
                                    .px_2()
                                    .text_xs()
                                    .text_color(fog())
                                    .child("Tags"),
                            )
                            .children(tag_shelves),
                    )
                    .child(
                        v_flex()
                            .id("tone-list")
                            .flex_1()
                            .h_full()
                            .overflow_y_scroll()
                            .p_2()
                            .gap_px()
                            .when(rows.is_empty(), |list| {
                                list.child(
                                    div()
                                        .p_3()
                                        .text_sm()
                                        .text_color(fog())
                                        .child("No tones here yet."),
                                )
                            })
                            .children(rows),
                    ),
            )
            .child(self.render_explorer_footer(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::{Lists, RECENT, Shelf, remember_recent, visible};
    use amp_preset::{Preset, PresetEntry};
    use std::path::PathBuf;

    fn entry(name: &str, tags: &[&str], notes: &str) -> PresetEntry {
        let mut preset = Preset::starter(name);
        preset.tags = tags.iter().map(|tag| (*tag).to_owned()).collect();
        preset.notes = notes.to_owned();
        PresetEntry {
            path: PathBuf::new(),
            preset,
        }
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn shelves_sort_tones_and_search_narrows_them() {
        let entries = [
            entry("Moss Clean", &["clean"], "warm chords"),
            entry("Bark Rhythm", &["high gain"], "tight mutes"),
            entry("My Lead", &["lead"], "singing"),
        ];
        let favorites = names(&["Bark Rhythm"]);
        let recent = names(&["My Lead", "Moss Clean"]);
        let factory = names(&["Moss Clean", "Bark Rhythm"]);
        let lists = Lists {
            favorites: &favorites,
            recent: &recent,
            factory: &factory,
        };
        assert_eq!(visible(&entries, &Shelf::All, "", &lists), [0, 1, 2]);
        assert_eq!(visible(&entries, &Shelf::Favorites, "", &lists), [1]);
        assert_eq!(visible(&entries, &Shelf::Factory, "", &lists), [0, 1]);
        assert_eq!(visible(&entries, &Shelf::Mine, "", &lists), [2]);
        assert_eq!(
            visible(&entries, &Shelf::Tag("clean".to_owned()), "", &lists),
            [0]
        );
        assert_eq!(visible(&entries, &Shelf::Recent, "", &lists), [2, 0]);
        assert_eq!(visible(&entries, &Shelf::All, "GAIN tight", &lists), [1]);
        assert!(visible(&entries, &Shelf::Mine, "moss", &lists).is_empty());
    }

    #[test]
    fn the_recent_list_stays_short_with_no_repeats() {
        let mut recent = Vec::new();
        for index in 0..RECENT + 3 {
            remember_recent(&mut recent, &format!("Tone {index}"));
        }
        remember_recent(&mut recent, "Tone 7");
        assert_eq!(recent.len(), RECENT);
        assert_eq!(recent[0], "Tone 7");
        assert_eq!(recent.iter().filter(|name| *name == "Tone 7").count(), 1);
    }
}
