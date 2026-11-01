//! The Tone Explorer. Filters on the left, tones in the middle, and the loaded
//! tone on the right. Filters work like a shop's: several choices within a group
//! widen the list, a choice in another group narrows it, and what's applied stays
//! in view above the list. Tones load as they're clicked or arrowed to, so they
//! can be compared by ear with the guitar in hand.

use super::{Overlay, Rig, board::block_art};
use crate::{
    art::{self, PRINT_FONT},
    style::{fog, hairline, moss, moss_glow, sheet},
};
use amp_preset::{PresetEntry, factory_tones};
use gpui_kit::{
    component::{
        ActiveTheme, Sizable, StyledExt,
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
const FILTER_WIDTH: f32 = 200.0;
const DETAIL_WIDTH: f32 = 280.0;
const CHARACTER_WIDTH: f32 = 128.0;
const STYLE_WIDTH: f32 = 170.0;
const ROW_HEIGHT: f32 = 40.0;
/// Tags that say how much gain a tone has, from least to most.
const CHARACTER: [&str; 5] = ["clean", "edge of breakup", "crunch", "high gain", "fuzz"];
const ROLE: [&str; 2] = ["rhythm", "lead"];

#[derive(Clone, Debug, PartialEq)]
enum Shelf {
    All,
    Favorites,
    Recent,
    Factory,
    Mine,
}

/// The groups tags are filtered in. Everything that isn't a known character or
/// role counts as a style, so a tag from a shared tone still has a home.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Group {
    Character,
    Role,
    Style,
}

fn group_of(tag: &str) -> Group {
    if CHARACTER.contains(&tag) {
        Group::Character
    } else if ROLE.contains(&tag) {
        Group::Role
    } else {
        Group::Style
    }
}

/// What the Explorer is showing.
pub(super) struct Explorer {
    pub(super) search: Entity<InputState>,
    shelf: Shelf,
    chosen: BTreeSet<String>,
    factory: Vec<String>,
}

impl Explorer {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Rig>) -> Self {
        Self {
            search: cx.new(|cx| InputState::new(window, cx).placeholder("Search tones")),
            shelf: Shelf::All,
            chosen: BTreeSet::new(),
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
    }
}

/// Any tag within a group will do, and every group with a choice has to match.
fn matches_tags(entry: &PresetEntry, chosen: &BTreeSet<String>) -> bool {
    [Group::Character, Group::Role, Group::Style]
        .iter()
        .all(|group| {
            let mut wanted = chosen.iter().filter(|tag| group_of(tag) == *group);
            let open = wanted.clone().next().is_none();
            open || wanted.any(|tag| entry.preset.tags.contains(tag))
        })
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
fn visible(
    entries: &[PresetEntry],
    shelf: &Shelf,
    chosen: &BTreeSet<String>,
    search: &str,
    lists: &Lists,
) -> Vec<usize> {
    let mut found: Vec<usize> = (0..entries.len())
        .filter(|index| {
            let entry = &entries[*index];
            on_shelf(entry, shelf, lists)
                && matches_tags(entry, chosen)
                && matches_search(entry, search)
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

/// A tone's character, or a dash for a tone that has none.
fn character_of(tags: &[String]) -> String {
    tags.iter()
        .find(|tag| group_of(tag) == Group::Character)
        .map_or_else(|| "\u{2013}".to_owned(), |tag| sentence_case(tag))
}

/// A tone's style and role, for the list.
fn style_of(tags: &[String]) -> String {
    tags.iter()
        .filter(|tag| group_of(tag) != Group::Character)
        .map(|tag| sentence_case(tag))
        .collect::<Vec<_>>()
        .join(", ")
}

fn sentence_case(tag: &str) -> String {
    let mut letters = tag.chars();
    letters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(letters).collect()
    })
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
            // Typing is the quickest way in, so the search box starts ready.
            let search = self.explorer.search.read(cx).focus_handle(cx);
            search.focus(window, cx);
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

    fn search_text(&self, cx: &App) -> String {
        self.explorer.search.read(cx).value().to_string()
    }

    fn visible_tones(&self, cx: &App) -> Vec<usize> {
        visible(
            &self.presets,
            &self.explorer.shelf,
            &self.explorer.chosen,
            &self.search_text(cx),
            &self.lists(),
        )
    }

    /// Arrows move through the list and load as they go.
    pub(super) fn step_explorer(
        &mut self,
        step: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlay != Overlay::Tones {
            return;
        }
        let tones = self.visible_tones(cx);
        if tones.is_empty() {
            return;
        }
        let current = tones
            .iter()
            .position(|index| self.presets[*index].preset.name == self.preset.name);
        let next = current.map_or(0, |at| {
            (at as isize + step).clamp(0, tones.len() as isize - 1) as usize
        });
        let preset = self.presets[tones[next]].preset.clone();
        self.load_preset(preset, window, cx);
    }

    fn toggle_tag(&mut self, tag: &str, cx: &mut Context<Self>) {
        if !self.explorer.chosen.remove(tag) {
            self.explorer.chosen.insert(tag.to_owned());
        }
        cx.notify();
    }

    fn render_shelf(
        &self,
        id: usize,
        title: &'static str,
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
            .px_2()
            .h(px(30.))
            .rounded_md()
            .text_sm()
            .cursor_pointer()
            .when(current, |row| {
                row.bg(moss_glow(0.14)).text_color(moss()).font_semibold()
            })
            .when(!current, |row| row.hover(|row| row.bg(moss_glow(0.06))))
            .child(title)
            .child(div().text_xs().text_color(fog()).child(count.to_string()))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.explorer.shelf = shelf.clone();
                cx.notify();
            }))
    }

    /// A tag filter. Its count is how many tones a press would leave showing.
    fn render_tag(
        &self,
        id: usize,
        tag: String,
        search: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let chosen = self.explorer.chosen.contains(&tag);
        let mut with_tag = self.explorer.chosen.clone();
        with_tag.insert(tag.clone());
        let count = visible(
            &self.presets,
            &self.explorer.shelf,
            &with_tag,
            search,
            &self.lists(),
        )
        .len();
        let empty = count == 0 && !chosen;
        h_flex()
            .id(("tag", id))
            .gap_2()
            .items_center()
            .px_2()
            .h(px(28.))
            .rounded_md()
            .text_sm()
            .when(empty, |row| row.opacity(0.4))
            .when(!empty, |row| {
                row.cursor_pointer().hover(|row| row.bg(moss_glow(0.06)))
            })
            .child(
                div()
                    .size(px(14.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .border_1()
                    .border_color(if chosen { moss() } else { fog() })
                    .when(chosen, |check| {
                        check
                            .bg(moss())
                            .text_size(px(10.))
                            .text_color(cx.theme().primary_foreground)
                            .child("\u{2713}")
                    }),
            )
            .child(div().flex_1().truncate().child(sentence_case(&tag)))
            .child(div().text_xs().text_color(fog()).child(count.to_string()))
            .when(!empty, |row| {
                row.on_click(cx.listener(move |this, _, _, cx| this.toggle_tag(&tag, cx)))
            })
    }

    fn render_filters(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let search = self.search_text(cx);
        let tags: BTreeSet<String> = self
            .presets
            .iter()
            .flat_map(|entry| entry.preset.tags.iter().cloned())
            .collect();
        let heading = |title: &'static str| {
            div()
                .mt_4()
                .mb_1()
                .px_2()
                .text_xs()
                .font_semibold()
                .text_color(fog())
                .child(title)
        };
        let shelves: Vec<AnyElement> = [
            ("All tones", Shelf::All),
            ("Favorites", Shelf::Favorites),
            ("Recent", Shelf::Recent),
            ("My tones", Shelf::Mine),
            ("Factory", Shelf::Factory),
        ]
        .into_iter()
        .enumerate()
        .map(|(id, (title, shelf))| self.render_shelf(id, title, shelf, cx).into_any_element())
        .collect();
        // Gain runs low to high, as players think of it, and styles are alphabetical.
        let ordered: Vec<String> = CHARACTER
            .iter()
            .chain(ROLE.iter())
            .filter(|tag| tags.contains(**tag))
            .map(|tag| (*tag).to_owned())
            .chain(
                tags.iter()
                    .filter(|tag| group_of(tag) == Group::Style)
                    .cloned(),
            )
            .collect();
        let group_rows = |group: Group, cx: &mut Context<Self>| -> Vec<AnyElement> {
            ordered
                .iter()
                .enumerate()
                .filter(|(_, tag)| group_of(tag) == group)
                .map(|(id, tag)| {
                    self.render_tag(id, tag.clone(), &search, cx)
                        .into_any_element()
                })
                .collect()
        };
        let character = group_rows(Group::Character, cx);
        let role = group_rows(Group::Role, cx);
        let style = group_rows(Group::Style, cx);
        v_flex()
            .id("filters")
            .w(px(FILTER_WIDTH))
            .h_full()
            .flex_shrink_0()
            .overflow_y_scroll()
            .p_2()
            .border_r_1()
            .border_color(hairline())
            .children(shelves)
            .when(!character.is_empty(), |column| {
                column.child(heading("Character")).children(character)
            })
            .when(!role.is_empty(), |column| {
                column.child(heading("Role")).children(role)
            })
            .when(!style.is_empty(), |column| {
                column.child(heading("Style")).children(style)
            })
    }

    fn render_tone_row(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let preset = self.presets[index].preset.clone();
        let current = preset.name == self.preset.name;
        let favorite = self.is_favorite(&preset.name);
        let star_name = preset.name.clone();
        h_flex()
            .id(("tone", index))
            .flex_shrink_0()
            .h(px(ROW_HEIGHT))
            .gap_2()
            .items_center()
            .pr_3()
            .rounded_md()
            .border_b_1()
            .border_color(hairline())
            .cursor_pointer()
            .when(current, |row| row.bg(moss_glow(0.14)))
            .when(!current, |row| row.hover(|row| row.bg(moss_glow(0.06))))
            .child(
                div()
                    .w(px(3.))
                    .h(px(24.))
                    .rounded_sm()
                    .when(current, |mark| mark.bg(moss())),
            )
            .child(
                div()
                    .id(("star", index))
                    .w(px(20.))
                    .text_color(if favorite { moss() } else { fog() })
                    .child(if favorite { "\u{2605}" } else { "\u{2606}" })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_favorite(&star_name, cx);
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_semibold()
                    .when(current, |name| name.text_color(moss()))
                    .child(preset.name.clone()),
            )
            .child(
                div()
                    .w(px(CHARACTER_WIDTH))
                    .flex_shrink_0()
                    .text_sm()
                    .truncate()
                    .child(character_of(&preset.tags)),
            )
            .child(
                div()
                    .w(px(STYLE_WIDTH))
                    .flex_shrink_0()
                    .text_sm()
                    .truncate()
                    .text_color(fog())
                    .child(style_of(&preset.tags)),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if event.click_count() >= 2 {
                    this.close_overlay(window, cx);
                } else {
                    this.load_preset(preset.clone(), window, cx);
                }
            }))
    }

    /// What's applied, each removable, so the list explains itself.
    fn render_applied(&self, count: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let chips: Vec<_> = self
            .explorer
            .chosen
            .iter()
            .cloned()
            .enumerate()
            .map(|(id, tag)| {
                h_flex()
                    .id(("applied", id))
                    .gap_1()
                    .items_center()
                    .px_2()
                    .h(px(24.))
                    .rounded_md()
                    .bg(moss_glow(0.14))
                    .text_xs()
                    .cursor_pointer()
                    .child(sentence_case(&tag))
                    .child(div().text_color(fog()).child("\u{00D7}"))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_tag(&tag, cx)))
            })
            .collect();
        let filtered = !self.explorer.chosen.is_empty();
        h_flex()
            .flex_shrink_0()
            .flex_wrap()
            .gap_2()
            .items_center()
            .px_3()
            .min_h(px(36.))
            .child(div().text_sm().text_color(fog()).child(if count == 1 {
                "1 tone".to_owned()
            } else {
                format!("{count} tones")
            }))
            .children(chips)
            .when(filtered, |row| {
                row.child(
                    Button::new("clear-filters")
                        .small()
                        .ghost()
                        .label("Clear all")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.explorer.chosen.clear();
                            cx.notify();
                        })),
                )
            })
    }

    fn render_tone_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tones = self.visible_tones(cx);
        let count = tones.len();
        let rows: Vec<AnyElement> = tones
            .iter()
            .map(|index| self.render_tone_row(*index, cx).into_any_element())
            .collect();
        let column =
            |title: &'static str, width: f32| div().w(px(width)).flex_shrink_0().child(title);
        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(
                div()
                    .flex_shrink_0()
                    .px_3()
                    .pt_3()
                    .child(Input::new(&self.explorer.search)),
            )
            .child(self.render_applied(count, cx))
            .child(
                h_flex()
                    .flex_shrink_0()
                    .gap_2()
                    .pl(px(33.))
                    .pr_6()
                    .pb_1()
                    .border_b_1()
                    .border_color(hairline())
                    .text_xs()
                    .font_semibold()
                    .text_color(fog())
                    .child(div().flex_1().child("Tone"))
                    .child(column("Character", CHARACTER_WIDTH))
                    .child(column("Style and role", STYLE_WIDTH)),
            )
            .child(
                v_flex()
                    .id("tone-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_3()
                    .when(rows.is_empty(), |list| {
                        list.child(
                            div()
                                .p_4()
                                .text_color(fog())
                                .child("No tones match. Try removing a filter."),
                        )
                    })
                    .children(rows),
            )
    }

    /// The loaded tone, in full. Its picture comes from the tone when it has
    /// one, and from its amp when it doesn't.
    fn render_tone_detail(&self) -> impl IntoElement {
        let preset = &self.preset;
        let picture = preset
            .blocks
            .iter()
            .filter_map(|block| block.origin.as_ref())
            .find_map(|origin| self.tone_image(origin));
        let amp = preset
            .blocks
            .iter()
            .enumerate()
            .find(|(_, block)| block.kind.is_amp())
            .map(|(slot, block)| block_art(block, self.block_name(slot)));
        let chain: Vec<_> = preset
            .blocks
            .iter()
            .enumerate()
            .map(|(slot, block)| {
                let art = block_art(block, self.block_name(slot));
                h_flex()
                    .justify_between()
                    .gap_2()
                    .text_sm()
                    .when(!block.enabled, |row| row.opacity(0.45))
                    .child(div().truncate().child(art.title))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(fog())
                            .child(art.type_label),
                    )
            })
            .collect();
        v_flex()
            .id("tone-detail")
            .w(px(DETAIL_WIDTH))
            .h_full()
            .flex_shrink_0()
            .overflow_y_scroll()
            .gap_3()
            .p_4()
            .border_l_1()
            .border_color(hairline())
            .child(
                div()
                    .h(px(110.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .map(|frame| match (picture, amp) {
                        (Some(image), _) => frame.child(
                            img(image)
                                .size(px(110.))
                                .rounded_md()
                                .object_fit(ObjectFit::Cover),
                        ),
                        (None, Some(amp)) => frame.child(art::thumb(&amp, true)),
                        (None, None) => frame,
                    }),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .font_family(PRINT_FONT)
                            .text_lg()
                            .font_semibold()
                            .child(preset.name.clone()),
                    )
                    .child(
                        div().text_xs().text_color(fog()).child(
                            preset
                                .tags
                                .iter()
                                .map(|tag| sentence_case(tag))
                                .collect::<Vec<_>>()
                                .join(", "),
                        ),
                    ),
            )
            .when(!preset.notes.is_empty(), |column| {
                column.child(div().text_sm().child(preset.notes.clone()))
            })
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_semibold()
                            .text_color(fog())
                            .child("Signal chain"),
                    )
                    .children(chain),
            )
    }

    fn render_explorer_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .flex_shrink_0()
            .gap_2()
            .items_center()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(hairline())
            .child(div().text_sm().text_color(fog()).child("Save as"))
            .child(
                div()
                    .w(px(240.))
                    .child(Input::new(&self.preset_name).small()),
            )
            .child(
                Button::new("save-as")
                    .small()
                    .primary()
                    .label("Save")
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
        v_flex()
            .id("tone-explorer")
            .occlude()
            .absolute()
            .top(px(8.))
            .bottom(px(8.))
            .left(px(16.))
            .right(px(16.))
            .rounded_lg()
            .border_1()
            .border_color(hairline())
            .bg(sheet())
            .child(
                h_flex()
                    .flex_shrink_0()
                    .justify_between()
                    .items_center()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(hairline())
                    .child(div().font_semibold().child("Tone Explorer"))
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
                    .child(self.render_filters(cx))
                    .child(self.render_tone_list(cx))
                    .child(self.render_tone_detail()),
            )
            .child(self.render_explorer_footer(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Lists, RECENT, Shelf, character_of, remember_recent, sentence_case, style_of, visible,
    };
    use amp_preset::{Preset, PresetEntry};
    use std::{collections::BTreeSet, path::PathBuf};

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

    fn tags(list: &[&str]) -> BTreeSet<String> {
        list.iter().map(|tag| (*tag).to_owned()).collect()
    }

    fn tones() -> [PresetEntry; 4] {
        [
            entry("Moss Clean", &["clean"], "warm chords"),
            entry("Bark Rhythm", &["high gain", "rhythm"], "tight mutes"),
            entry("Peat Lead", &["high gain", "lead"], "singing"),
            entry("Silver", &["clean", "blues", "lead"], "glassy"),
        ]
    }

    #[test]
    fn shelves_sort_tones_and_search_narrows_them() {
        let entries = tones();
        let favorites = names(&["Bark Rhythm"]);
        let recent = names(&["Silver", "Moss Clean"]);
        let factory = names(&["Moss Clean", "Bark Rhythm"]);
        let lists = Lists {
            favorites: &favorites,
            recent: &recent,
            factory: &factory,
        };
        let none = BTreeSet::new();
        let show = |shelf, search| visible(&entries, &shelf, &none, search, &lists);
        assert_eq!(show(Shelf::All, ""), [0, 1, 2, 3]);
        assert_eq!(show(Shelf::Favorites, ""), [1]);
        assert_eq!(show(Shelf::Factory, ""), [0, 1]);
        assert_eq!(show(Shelf::Mine, ""), [2, 3]);
        assert_eq!(show(Shelf::Recent, ""), [3, 0]);
        assert_eq!(show(Shelf::All, "GAIN tight"), [1]);
        assert!(show(Shelf::Mine, "moss").is_empty());
    }

    #[test]
    fn choices_in_a_group_widen_and_groups_narrow() {
        let entries = tones();
        let lists = Lists {
            favorites: &[],
            recent: &[],
            factory: &[],
        };
        let show = |chosen: &[&str]| visible(&entries, &Shelf::All, &tags(chosen), "", &lists);
        assert_eq!(show(&["clean"]), [0, 3]);
        assert_eq!(show(&["clean", "high gain"]), [0, 1, 2, 3]);
        assert_eq!(show(&["high gain", "lead"]), [2]);
        assert_eq!(show(&["clean", "blues"]), [3]);
    }

    #[test]
    fn tags_are_sorted_into_columns() {
        let tags = names(&["neo-soul", "clean", "rhythm"]);
        assert_eq!(character_of(&tags), "Clean");
        assert_eq!(style_of(&tags), "Neo-soul, Rhythm");
        assert_eq!(character_of(&[]), "\u{2013}");
        assert_eq!(sentence_case("edge of breakup"), "Edge of breakup");
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
