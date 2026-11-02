//! The Tone Explorer. Search and two rows of pills on top, tones below, and the
//! loaded tone on the right. The list is short, so filters stay simple: each row
//! picks one thing, pressing it again clears it, and a lit pill is all the
//! explaining a filter needs. Tones load as they're clicked or arrowed to, so they
//! can be compared by ear with the guitar in hand.

use super::{Overlay, Rig, board::block_art, close_button};
use crate::{
    art::{self, PRINT_FONT},
    style::{fog, hairline, moss, moss_glow, sheet},
};
use amp_preset::{PresetEntry, factory_tones};
use gpui_kit::{
    component::{
        Sizable, StyledExt,
        button::{Button, ButtonVariants},
        h_flex,
        input::{Input, InputState},
        menu::{DropdownMenu, PopupMenuItem},
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use std::collections::BTreeSet;

/// How many tones the Recent shelf keeps.
const RECENT: usize = 12;
const DETAIL_WIDTH: f32 = 280.0;
const ROW_HEIGHT: f32 = 40.0;
const PILL_HEIGHT: f32 = 26.0;
const SHELVES: [(&str, Shelf); 4] = [
    ("All", Shelf::All),
    ("Favorites", Shelf::Favorites),
    ("Recent", Shelf::Recent),
    ("My tones", Shelf::Mine),
];
/// Tags that say how much gain a tone has, from least to most.
const CHARACTER: [&str; 5] = ["clean", "edge of breakup", "crunch", "high gain", "fuzz"];
const ROLE: [&str; 2] = ["rhythm", "lead"];

/// Something the Library menu can run.
type LibraryAction = fn(&mut Rig, &mut Window, &mut Context<Rig>);

#[derive(Clone, Debug, PartialEq)]
enum Shelf {
    All,
    Favorites,
    Recent,
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
    /// Whether the role and style pills are showing.
    more: bool,
    /// The tone waiting for a second press to delete it.
    deleting: Option<String>,
    factory: Vec<String>,
}

impl Explorer {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Rig>) -> Self {
        Self {
            search: cx.new(|cx| InputState::new(window, cx).placeholder("Search tones")),
            shelf: Shelf::All,
            chosen: BTreeSet::new(),
            more: false,
            deleting: None,
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
        Shelf::Mine => !lists.factory.contains(name),
    }
}

/// Picks `tag`, which takes over from anything else in its group. Picking it
/// again clears it.
fn choose(chosen: &mut BTreeSet<String>, tag: &str) {
    let was_chosen = chosen.contains(tag);
    let group = group_of(tag);
    chosen.retain(|other| group_of(other) != group);
    if !was_chosen {
        chosen.insert(tag.to_owned());
    }
}

/// A tone has to carry everything that's picked.
fn matches_tags(entry: &PresetEntry, chosen: &BTreeSet<String>) -> bool {
    chosen.iter().all(|tag| entry.preset.tags.contains(tag))
}

/// Every word of the search has to appear in the name, tags, notes, or gear, so
/// a capture's name finds the tones that use it.
fn matches_search(entry: &PresetEntry, search: &str) -> bool {
    let preset = &entry.preset;
    let gear = preset.blocks.iter().map(|block| {
        block
            .origin
            .as_ref()
            .map(|origin| origin.title.as_str())
            .or(block.asset.as_ref().map(|asset| asset.name.as_str()))
            .unwrap_or(block.kind.title())
    });
    let gear = gear.collect::<Vec<_>>().join(" ");
    let text = format!(
        "{} {} {} {gear}",
        preset.name,
        preset.tags.join(" "),
        preset.notes
    )
    .to_lowercase();
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

/// A tone's tags for the list, gain first, since that's what people scan for.
fn tag_line(tags: &[String]) -> String {
    let in_group = |group| tags.iter().filter(move |tag| group_of(tag) == group);
    in_group(Group::Character)
        .chain(in_group(Group::Role))
        .chain(in_group(Group::Style))
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
        choose(&mut self.explorer.chosen, tag);
        cx.notify();
    }

    /// The tags in `group` a press would leave tones for, in the order to show
    /// them: gain runs low to high, as players think of it. What's picked always
    /// stays, so it can be cleared.
    fn pill_tags(&self, group: Group, search: &str) -> Vec<String> {
        let known: BTreeSet<&String> = self
            .presets
            .iter()
            .flat_map(|entry| &entry.preset.tags)
            .filter(|tag| group_of(tag) == group)
            .collect();
        let ordered: Vec<String> = match group {
            Group::Character => CHARACTER.map(str::to_owned).into(),
            Group::Role => ROLE.map(str::to_owned).into(),
            Group::Style => known.iter().map(|tag| (*tag).clone()).collect(),
        };
        ordered
            .into_iter()
            .filter(|tag| known.contains(tag))
            .filter(|tag| {
                let chosen = &self.explorer.chosen;
                let mut with_tag = chosen.clone();
                choose(&mut with_tag, tag);
                chosen.contains(tag)
                    || !visible(
                        &self.presets,
                        &self.explorer.shelf,
                        &with_tag,
                        search,
                        &self.lists(),
                    )
                    .is_empty()
            })
            .collect()
    }

    fn render_shelf(
        &self,
        id: usize,
        title: &'static str,
        shelf: Shelf,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        pill(("shelf", id), title, self.explorer.shelf == shelf).on_click(cx.listener(
            move |this, _, _, cx| {
                this.explorer.shelf = shelf.clone();
                cx.notify();
            },
        ))
    }

    fn render_tag(&self, id: usize, tag: String, cx: &mut Context<Self>) -> impl IntoElement {
        let chosen = self.explorer.chosen.contains(&tag);
        pill(("tag", id), sentence_case(&tag), chosen)
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_tag(&tag, cx)))
    }

    fn render_tag_row(
        &self,
        first_id: usize,
        tags: Vec<String>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        tags.into_iter()
            .enumerate()
            .map(|(id, tag)| self.render_tag(first_id + id, tag, cx).into_any_element())
            .collect()
    }

    /// The shelves, gain, and behind More, the rarer filters.
    fn render_filters(&self, count: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let search = self.search_text(cx);
        let shelves: Vec<AnyElement> = SHELVES
            .into_iter()
            .enumerate()
            .map(|(id, (title, shelf))| self.render_shelf(id, title, shelf, cx).into_any_element())
            .collect();
        let gain = self.pill_tags(Group::Character, &search);
        let role = self.pill_tags(Group::Role, &search);
        let style = self.pill_tags(Group::Style, &search);
        let has_more = !role.is_empty() || !style.is_empty();
        let rare_chosen = self
            .explorer
            .chosen
            .iter()
            .any(|tag| group_of(tag) != Group::Character);
        let show_more = self.explorer.more || rare_chosen;
        let filtered = !self.explorer.chosen.is_empty();
        let gain_pills = self.render_tag_row(0, gain, cx);
        let mut rare_pills = self.render_tag_row(CHARACTER.len(), role, cx);
        rare_pills.extend(self.render_tag_row(CHARACTER.len() + ROLE.len(), style, cx));
        v_flex()
            .flex_shrink_0()
            .gap_2()
            .px_3()
            .py_2()
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .children(shelves)
                    .child(div().flex_1())
                    .child(div().text_sm().text_color(fog()).child(if count == 1 {
                        "1 tone".to_owned()
                    } else {
                        format!("{count} tones")
                    })),
            )
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .flex_wrap()
                    .children(gain_pills)
                    .when(has_more, |row| {
                        row.child(
                            pill("more-filters", "More", show_more).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.explorer.more = !this.explorer.more;
                                    cx.notify();
                                },
                            )),
                        )
                    })
                    .when(filtered, |row| {
                        row.child(
                            Button::new("clear-filters")
                                .small()
                                .ghost()
                                .label("Clear")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.explorer.chosen.clear();
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .when(show_more && !rare_pills.is_empty(), |column| {
                column.child(h_flex().gap_1().flex_wrap().children(rare_pills))
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
                    .flex_shrink_0()
                    .font_semibold()
                    .when(current, |name| name.text_color(moss()))
                    .child(preset.name.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_right()
                    .truncate()
                    .text_color(fog())
                    .child(tag_line(&preset.tags)),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if event.click_count() >= 2 {
                    this.close_overlay(window, cx);
                } else {
                    this.load_preset(preset.clone(), window, cx);
                }
            }))
    }

    fn render_tone_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tones = self.visible_tones(cx);
        let count = tones.len();
        let rows: Vec<AnyElement> = tones
            .iter()
            .map(|index| self.render_tone_row(*index, cx).into_any_element())
            .collect();
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
            .child(self.render_filters(count, cx))
            .child(
                v_flex()
                    .id("tone-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_3()
                    .pb_2()
                    .when(rows.is_empty(), |list| {
                        list.child(
                            v_flex()
                                .gap_2()
                                .items_start()
                                .p_4()
                                .child(
                                    div()
                                        .text_color(fog())
                                        .child("No tones match. Try clearing a filter."),
                                )
                                .child(
                                    Button::new("ask-assistant")
                                        .small()
                                        .outline()
                                        .label("Ask the assistant")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.ask_about_search(window, cx);
                                        })),
                                ),
                        )
                    })
                    .children(rows),
            )
    }

    /// The loaded tone, in full. Its picture comes from the tone when it has
    /// one, and from its amp when it doesn't.
    fn render_tone_detail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let preset = &self.preset;
        let saved = self
            .presets
            .iter()
            .any(|entry| entry.preset.name == preset.name);
        let deleting = self.explorer.deleting.as_deref() == Some(preset.name.as_str());
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
            .when(saved && !deleting, |column| {
                column.child(
                    Button::new("delete-tone")
                        .small()
                        .ghost()
                        .label("Delete tone")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.explorer.deleting = Some(this.preset.name.clone());
                            cx.notify();
                        })),
                )
            })
            .when(saved && deleting, |column| {
                column.child(
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new("confirm-delete")
                                .small()
                                .danger()
                                .label("Delete it")
                                .on_click(cx.listener(|this, _, _, cx| this.delete_tone(cx))),
                        )
                        .child(
                            Button::new("cancel-delete")
                                .small()
                                .ghost()
                                .label("Keep it")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.explorer.deleting = None;
                                    cx.notify();
                                })),
                        ),
                )
            })
    }

    /// Carries what was searched for over to the assistant, ready to send.
    fn ask_about_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let search = self.search_text(cx);
        self.close_overlay(window, cx);
        if !self.assistant.open {
            self.toggle_assistant(window, cx);
        }
        if !search.is_empty() {
            self.assistant.input.update(cx, |input, cx| {
                input.set_value(format!("Make me a tone like: {search}"), window, cx);
            });
        }
    }

    /// Takes the loaded tone out of the library. It stays on the rig, marked as
    /// unsaved, so a mistake is one Save away from undone.
    fn delete_tone(&mut self, cx: &mut Context<Self>) {
        let name = self.preset.name.clone();
        self.explorer.deleting = None;
        if let Err(error) = self.library.delete_preset(&name) {
            return self.set_error(error, cx);
        }
        self.settings.favorites.retain(|tone| *tone != name);
        self.settings.recent.retain(|tone| *tone != name);
        self.saved = None;
        self.save_settings(cx);
        self.refresh_presets();
        cx.notify();
    }

    fn render_explorer_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let rig = cx.weak_entity();
        h_flex()
            .flex_shrink_0()
            .gap_2()
            .items_center()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(hairline())
            .child(div().text_sm().text_color(fog()).child("Name"))
            .child(
                div()
                    .w(px(240.))
                    .child(Input::new(&self.preset_name).small()),
            )
            .child(
                Button::new("save-as")
                    .small()
                    .primary()
                    .label("Save as new")
                    .tooltip("Keeps the tone you started from")
                    .on_click(cx.listener(|this, _, window, cx| this.save_as_new(window, cx))),
            )
            .child(div().flex_1())
            .child(
                Button::new("library-menu")
                    .small()
                    .ghost()
                    .label("Library")
                    .dropdown_menu_with_anchor(Anchor::BottomRight, move |menu, _, _| {
                        let item = |label: &'static str, run: LibraryAction| {
                            let rig = rig.clone();
                            PopupMenuItem::new(label).on_click(move |_, window, cx| {
                                let _ = rig.update(cx, |this, cx| run(this, window, cx));
                            })
                        };
                        menu.item(item("Export this tone", Rig::export_pack))
                            .item(item("Import tones", Rig::import_pack))
                            .item(item("Change library folder", Rig::choose_library))
                    }),
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
                    .child(close_button("close-explorer", cx)),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .child(self.render_tone_list(cx))
                    .child(self.render_tone_detail(cx)),
            )
            .child(self.render_explorer_footer(cx))
    }
}

/// A filter you can see the state of. It's lit while applied.
pub(super) fn pill(
    id: impl Into<ElementId>,
    title: impl Into<SharedString>,
    lit: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .flex_shrink_0()
        .h(px(PILL_HEIGHT))
        .px_3()
        .rounded_full()
        .border_1()
        .text_sm()
        .cursor_pointer()
        .when(lit, |pill| {
            pill.bg(moss_glow(0.16))
                .border_color(moss())
                .text_color(moss())
        })
        .when(!lit, |pill| {
            pill.border_color(hairline())
                .hover(|pill| pill.bg(moss_glow(0.06)))
        })
        .child(title.into())
}

#[cfg(test)]
mod tests {
    use super::{Lists, RECENT, Shelf, choose, remember_recent, sentence_case, tag_line, visible};
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
        assert_eq!(show(Shelf::Mine, ""), [2, 3]);
        assert_eq!(show(Shelf::Recent, ""), [3, 0]);
        assert_eq!(show(Shelf::All, "GAIN tight"), [1]);
        assert!(show(Shelf::Mine, "moss").is_empty());
    }

    #[test]
    fn picks_replace_within_a_group_and_narrow_across_groups() {
        let entries = tones();
        let lists = Lists {
            favorites: &[],
            recent: &[],
            factory: &[],
        };
        let show = |chosen: &[&str]| visible(&entries, &Shelf::All, &tags(chosen), "", &lists);
        assert_eq!(show(&["clean"]), [0, 3]);
        assert_eq!(show(&["high gain", "lead"]), [2]);
        assert_eq!(show(&["clean", "blues"]), [3]);

        let mut chosen = tags(&["clean", "lead"]);
        choose(&mut chosen, "crunch");
        assert_eq!(chosen, tags(&["crunch", "lead"]));
        choose(&mut chosen, "crunch");
        assert_eq!(chosen, tags(&["lead"]));
    }

    #[test]
    fn search_finds_a_tone_by_the_gear_on_it() {
        let mut plexi = entry("Saturday", &[], "");
        plexi.preset.blocks[2].origin = Some(amp_preset::ToneOrigin {
            title: "Plexi 1959".into(),
            ..amp_preset::ToneOrigin::default()
        });
        let entries = [entry("Moss", &[], ""), plexi];
        let lists = Lists {
            favorites: &[],
            recent: &[],
            factory: &[],
        };
        let none = BTreeSet::new();
        assert_eq!(visible(&entries, &Shelf::All, &none, "plexi", &lists), [1]);
    }

    #[test]
    fn tags_read_gain_first() {
        let tags = names(&["neo-soul", "rhythm", "clean"]);
        assert_eq!(tag_line(&tags), "Clean, Rhythm, Neo-soul");
        assert_eq!(tag_line(&[]), "");
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
