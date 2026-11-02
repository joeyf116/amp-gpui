//! The chain editor: the one place where blocks are added, replaced, removed,
//! reordered, and switched on and off.
//!
//! The strip shows every block as a card in signal order, numbered from the
//! guitar, with what it does ("Overdrive", "Amp") printed above its own name.
//! Every gap between cards has a + that opens the gear picker right under the
//! strip, already pointed at that spot. The same picker replaces a block, so
//! there is one panel for choosing gear, wherever the change is going.

use super::{
    Rig,
    board::{BoardDrag, DragSource, Gear, block_art, capture_block_kind, landing, place_after},
    explorer::pill,
};
use crate::{
    art::{self, GearArt, Shape},
    style::{fog, glow, hairline, moss, moss_glow, pane, sheet, well},
};
use amp_dsp::BlockKind;
use amp_preset::CaptureEntry;
use gpui_kit::{
    base::spring,
    component::{
        ActiveTheme, Sizable, StyledExt,
        button::{Button, ButtonVariants},
        h_flex,
        tooltip::Tooltip,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};

pub(super) const STRIP_HEIGHT: f32 = 280.0;
/// The picker sits in the column with everything else, so opening it moves the
/// stage down rather than covering it. Kept short so the stage still fits.
pub(super) const PICKER_HEIGHT: f32 = 238.0;
const CARD_WIDTH: f32 = 152.0;
const CARD_HEIGHT: f32 = 190.0;
/// The room a unit's picture gets on its card.
const CARD_ART: (f32, f32) = (132.0, 104.0);
const ACTIONS_HEIGHT: f32 = 28.0;
const GAP_WIDTH: f32 = 48.0;
const PLUS_SIZE: f32 = 26.0;
const STRIPE: f32 = 4.0;
const OPTION_WIDTH: f32 = 116.0;
const OPTION_ART: (f32, f32) = (96.0, 56.0);
const BUILT_IN: [BlockKind; 8] = [
    BlockKind::Tube,
    BlockKind::Cab,
    BlockKind::Drive,
    BlockKind::Compressor,
    BlockKind::Gate,
    BlockKind::Chorus,
    BlockKind::Delay,
    BlockKind::Reverb,
];

/// Where gear chosen in the picker goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum PickAt {
    /// Where that kind of unit usually sits in a rig.
    Usual,
    /// Into this gap, counted from the guitar: 0 is before the first block.
    Gap(usize),
    /// In place of this block.
    Replace(usize),
}

/// Which gear the picker lists.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Shelf {
    All,
    Amps,
    Cabinets,
    Pedals,
    Captures,
}

impl Shelf {
    const ALL: [(Self, &'static str); 5] = [
        (Self::All, "All"),
        (Self::Amps, "Amps"),
        (Self::Cabinets, "Cabinets"),
        (Self::Pedals, "Pedals"),
        (Self::Captures, "My captures"),
    ];

    fn holds(self, gear: &Gear, art: &GearArt) -> bool {
        match self {
            Self::All => true,
            Self::Amps => matches!(art.shape, Shape::Head { .. }),
            Self::Cabinets => matches!(art.shape, Shape::Cabinet { .. }),
            Self::Pedals => art.shape.is_stomp(),
            Self::Captures => matches!(gear, Gear::Capture(_)),
        }
    }
}

/// The open gear picker.
#[derive(Clone, Copy)]
pub(super) struct Picker {
    pub at: PickAt,
    pub shelf: Shelf,
}

/// The line under a block's name: on, off, or why it can't play.
fn status(enabled: bool, problem: bool) -> &'static str {
    match (problem, enabled) {
        (true, _) => "File missing",
        (false, true) => "On",
        (false, false) => "Off",
    }
}

/// Says where a gap is, by what's either side of it.
fn gap_place(before: Option<&str>, after: Option<&str>) -> String {
    match (before, after) {
        (None, None) => "First block, between the guitar and the speakers".into(),
        (None, Some(after)) => format!("Before {after}, first after the guitar"),
        (Some(before), None) => format!("After {before}, last before the speakers"),
        (Some(before), Some(after)) => format!("Between {before} and {after}"),
    }
}

impl Rig {
    /// "3 · Overdrive" and the unit's own name, the way every view names a block.
    pub(super) fn block_label(&self, slot: usize) -> (String, SharedString) {
        let art = block_art(&self.preset.blocks[slot], self.block_name(slot));
        (format!("{} · {}", slot + 1, art.shape.role()), art.title)
    }

    fn short_label(&self, slot: usize) -> String {
        let (role, title) = self.block_label(slot);
        format!("{title} ({role})")
    }

    pub(super) fn open_gear(&mut self, at: PickAt, cx: &mut Context<Self>) {
        let shelf = self.gear_picker.map_or(Shelf::All, |picker| picker.shelf);
        if let PickAt::Replace(slot) = at {
            self.selected = Some(slot);
        }
        self.gear_picker = Some(Picker { at, shelf });
        cx.notify();
    }

    pub(super) fn close_gear(&mut self, cx: &mut Context<Self>) {
        self.gear_picker = None;
        cx.notify();
    }

    /// G opens the picker for the spot after the selected block, or where
    /// gear usually goes, and closes it again.
    pub(super) fn toggle_gear(&mut self, cx: &mut Context<Self>) {
        if self.gear_picker.is_some() {
            return self.close_gear(cx);
        }
        let at = self
            .selected
            .map_or(PickAt::Usual, |slot| PickAt::Gap(slot + 1));
        self.open_gear(at, cx);
    }

    fn set_shelf(&mut self, shelf: Shelf, cx: &mut Context<Self>) {
        if let Some(picker) = &mut self.gear_picker {
            picker.shelf = shelf;
            cx.notify();
        }
    }

    /// Puts the chosen gear where the picker points, then closes it.
    fn pick(&mut self, gear: &Gear, cx: &mut Context<Self>) {
        let Some(picker) = self.gear_picker else {
            return;
        };
        let block = gear.block();
        match picker.at {
            PickAt::Usual => {
                let slot = place_after(self, &block);
                self.insert_block(block, slot, cx);
            }
            PickAt::Gap(gap) => self.insert_block(block, gap, cx),
            PickAt::Replace(slot) => self.replace_with(slot, gear, cx),
        }
        self.gear_picker = None;
        cx.notify();
    }

    /// Puts other gear in a block's place. A capture of the same kind keeps
    /// the block's knobs, so captures can be compared fairly.
    fn replace_with(&mut self, slot: usize, gear: &Gear, cx: &mut Context<Self>) {
        let Some(current) = self.preset.blocks.get(slot) else {
            return;
        };
        if let Gear::Capture(capture) = gear
            && current.kind == capture_block_kind(capture.kind)
        {
            return self.swap_capture(slot, capture, cx);
        }
        self.remember();
        self.preset.replace_block(slot, gear.block());
        self.captures[slot] = None;
        self.problems[slot] = None;
        self.selected = Some(slot);
        self.rebuild_chain(cx);
    }

    fn drop_on_board(&mut self, drag: &BoardDrag, gap: usize, cx: &mut Context<Self>) {
        self.hover_gap = None;
        self.hover_swap = None;
        match &drag.source {
            DragSource::Rack(gear) => {
                self.insert_block(gear.block(), gap, cx);
                self.gear_picker = None;
            }
            DragSource::Board(from) => self.move_block(*from, landing(*from, gap), cx),
        }
    }

    fn set_hover_gap(&mut self, gap: usize, cx: &mut Context<Self>) {
        if self.hover_gap != Some(gap) || self.hover_swap.is_some() {
            self.hover_gap = Some(gap);
            self.hover_swap = None;
            cx.notify();
        }
    }

    fn set_hover_swap(&mut self, slot: usize, cx: &mut Context<Self>) {
        if self.hover_swap != Some(slot) {
            self.hover_swap = Some(slot);
            self.hover_gap = None;
            cx.notify();
        }
    }

    /// The capture in a drag from the picker, if it can swap into `slot`.
    fn swappable(&self, drag: &BoardDrag, slot: usize) -> Option<CaptureEntry> {
        let DragSource::Rack(Gear::Capture(capture)) = &drag.source else {
            return None;
        };
        let block = self.preset.blocks.get(slot)?;
        (block.kind == capture_block_kind(capture.kind)).then(|| (**capture).clone())
    }

    fn set_hover_unit(&mut self, slot: usize, hovered: bool, cx: &mut Context<Self>) {
        let next = if hovered {
            Some(slot)
        } else if self.hover_unit == Some(slot) {
            None
        } else {
            self.hover_unit
        };
        if next != self.hover_unit {
            self.hover_unit = next;
            cx.notify();
        }
    }

    fn set_hover_add(&mut self, gap: usize, hovered: bool, cx: &mut Context<Self>) {
        let next = if hovered {
            Some(gap)
        } else if self.hover_add == Some(gap) {
            None
        } else {
            self.hover_add
        };
        if next != self.hover_add {
            self.hover_add = next;
            cx.notify();
        }
    }

    /// The space between two blocks: a cable, a + to add a block there, and
    /// a landing spot while something is being dragged.
    fn render_gap(&self, gap: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let dragging = cx.has_active_drag();
        let landing = dragging && self.hover_gap == Some(gap);
        let picking = self
            .gear_picker
            .is_some_and(|picker| picker.at == PickAt::Gap(gap));
        let last = gap == self.preset.blocks.len();
        let show_plus = !dragging && (picking || last || self.hover_add == Some(gap));
        let width = spring(
            format!("chain-gap-{gap}"),
            if landing { CARD_WIDTH * 0.7 } else { 0.0 },
            cx.theme().motion_tokens().spring_control,
            window,
            cx,
        );
        let cable = 0.22 + (self.output_level * 3.0).min(0.7);
        let place = gap_place(
            gap.checked_sub(1)
                .map(|slot| self.short_label(slot))
                .as_deref(),
            (!last).then(|| self.short_label(gap)).as_deref(),
        );
        let tip = format!("Add a block here. {place}");
        if self.preset.blocks.is_empty() && !landing {
            return self.render_first_block(gap, cx).into_any_element();
        }
        div()
            .id(("gap", gap))
            .relative()
            .flex_shrink_0()
            .h(px(CARD_HEIGHT))
            .w(px(GAP_WIDTH + width))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                this.set_hover_add(gap, *hovered, cx);
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_gear(PickAt::Gap(gap), cx);
            }))
            .on_drag_move::<BoardDrag>(cx.listener(
                move |this, event: &DragMoveEvent<BoardDrag>, _, cx| {
                    if event.bounds.contains(&event.event.position) {
                        this.set_hover_gap(gap, cx);
                    }
                },
            ))
            .on_drop(cx.listener(move |this, drag: &BoardDrag, _, cx| {
                this.drop_on_board(drag, gap, cx);
            }))
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .child(if landing {
                div()
                    .h(px(CARD_HEIGHT - 8.))
                    .w(px((width - 4.0).max(2.0)))
                    .rounded_lg()
                    .border_1()
                    .border_dashed()
                    .border_color(moss())
                    .bg(moss_glow(0.08))
            } else {
                div()
                    .h(px(2.))
                    .w_full()
                    .bg(moss_glow(cable))
                    .shadow(vec![glow(cable * 0.5, 8.0)])
            })
            .when(show_plus, |gap| {
                gap.child(
                    div()
                        .absolute()
                        .size(px(PLUS_SIZE))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .border_1()
                        .text_sm()
                        .font_semibold()
                        .when(picking, |plus| {
                            plus.bg(moss())
                                .border_color(moss())
                                .text_color(hsla(0.0, 0.0, 0.05, 1.0))
                                .shadow(vec![glow(0.7, 12.0)])
                        })
                        .when(!picking, |plus| {
                            plus.bg(well()).border_color(moss()).text_color(moss())
                        })
                        .child("+"),
                )
            })
            .into_any_element()
    }

    /// An empty chain shows one big place to start, which still takes drops.
    fn render_first_block(&self, gap: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let picking = self.gear_picker.is_some();
        v_flex()
            .id("first-block")
            .flex_shrink_0()
            .mx_2()
            .w(px(CARD_WIDTH * 1.6))
            .h(px(CARD_HEIGHT))
            .items_center()
            .justify_center()
            .gap_1()
            .rounded_lg()
            .border_1()
            .border_dashed()
            .border_color(moss())
            .bg(if picking {
                moss_glow(0.1)
            } else {
                moss_glow(0.03)
            })
            .cursor_pointer()
            .hover(|card| card.bg(moss_glow(0.08)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_gear(PickAt::Gap(gap), cx);
            }))
            .on_drop(cx.listener(move |this, drag: &BoardDrag, _, cx| {
                this.drop_on_board(drag, gap, cx);
            }))
            .child(div().text_xl().text_color(moss()).child("+"))
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .child("Add your first block"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(fog())
                    .child("An amp, a cabinet, or a pedal"),
            )
    }

    /// A block's card: its number in the chain and what it does, the unit, its
    /// name, and whether it's on. Click to edit it, drag to move it.
    fn render_card(&self, slot: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let block = &self.preset.blocks[slot];
        let art = block_art(block, self.block_name(slot));
        let selected = self.selected == Some(slot);
        let replacing = self
            .gear_picker
            .is_some_and(|picker| picker.at == PickAt::Replace(slot));
        let lit = selected || replacing;
        let enabled = block.enabled;
        let problem = self.problems.get(slot).is_some_and(Option::is_some);
        let badge = block.origin.as_ref().map(|origin| self.tone_badge(origin));
        let swap_target = self.hover_swap == Some(slot) && cx.has_active_drag();
        let show_actions = (selected || self.hover_unit == Some(slot)) && !cx.has_active_drag();
        let warning = cx.theme().warning;
        let role = art.shape.role();
        let tint = art.shape.tint();
        let tip = format!(
            "{} {role}: {}. Click to edit, drag to move.",
            ordinal(slot + 1),
            art.title
        );
        let drag = BoardDrag {
            source: DragSource::Board(slot),
            art: art.clone(),
        };
        let power = div()
            .id(("power", slot))
            .flex()
            .items_center()
            .gap_1()
            .px_1p5()
            .h(px(18.))
            .rounded_full()
            .border_1()
            .text_xs()
            .cursor_pointer()
            .when(problem, |pill| {
                pill.border_color(warning).text_color(warning)
            })
            .when(!problem && enabled, |pill| {
                pill.border_color(moss()).text_color(moss())
            })
            .when(!problem && !enabled, |pill| {
                pill.border_color(hairline()).text_color(fog())
            })
            .child(div().size(px(6.)).rounded_full().map(|dot| {
                if problem {
                    dot.bg(warning)
                } else if enabled {
                    dot.bg(moss()).shadow(vec![glow(0.8, 6.0)])
                } else {
                    dot.bg(hairline())
                }
            }))
            .child(status(enabled, problem))
            .tooltip(move |window, cx| {
                Tooltip::new(if enabled {
                    "Turn off (B)"
                } else {
                    "Turn on (B)"
                })
                .build(window, cx)
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.toggle_block(slot, cx);
            }));
        let card = v_flex()
            .id(("card", slot))
            .relative()
            .w(px(CARD_WIDTH))
            .h(px(CARD_HEIGHT))
            .px_3()
            .pb_3()
            .pt(px(12.0 + STRIPE))
            .gap_2()
            .rounded_lg()
            .overflow_hidden()
            .border_1()
            .bg(if lit { card_lit() } else { card() })
            .border_color(if lit { moss() } else { hairline() })
            .when(lit, |card| card.shadow(vec![glow(0.45, 14.0)]))
            .when(swap_target, |card| {
                card.border_2()
                    .border_color(moss())
                    .shadow(vec![glow(0.7, 16.0)])
            })
            .when(!lit, |card| {
                card.hover(|card| card.border_color(moss_glow(0.5)))
            })
            .cursor_grab()
            // Each kind of gear wears its own color along the top of its card.
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(STRIPE))
                    .bg(tint.opacity(if enabled { 0.9 } else { 0.35 })),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_shrink_0()
                            .size(px(20.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_xs()
                            .font_semibold()
                            .when(lit, |number| {
                                number.bg(moss()).text_color(hsla(0.0, 0.0, 0.05, 1.0))
                            })
                            .when(!lit, |number| {
                                number.border_1().border_color(hairline()).text_color(fog())
                            })
                            .child((slot + 1).to_string()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .text_xs()
                            .font_semibold()
                            .truncate()
                            .text_color(tint)
                            .child(role.to_uppercase()),
                    ),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(art::thumb_fit(&art, enabled && !problem, CARD_ART))
                    .children(badge)
                    .when(problem, |picture| {
                        picture.child(
                            div()
                                .absolute()
                                .top_0()
                                .right_0()
                                .size(px(16.))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(warning)
                                .text_color(hsla(0.0, 0.0, 0.05, 1.0))
                                .text_xs()
                                .font_semibold()
                                .child("!"),
                        )
                    }),
            )
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .truncate()
                    .text_color(if enabled {
                        cx.theme().foreground
                    } else {
                        fog()
                    })
                    .child(art.title.clone()),
            )
            .child(h_flex().child(power));
        v_flex()
            .id(("unit", slot))
            .flex_shrink_0()
            .gap_1()
            .items_center()
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                this.set_hover_unit(slot, *hovered, cx);
            }))
            .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
            .on_drag_move::<BoardDrag>(cx.listener(
                move |this, event: &DragMoveEvent<BoardDrag>, _, cx| {
                    if !event.bounds.contains(&event.event.position) {
                        return;
                    }
                    // A capture held over a block of its own kind swaps into it.
                    if this.swappable(event.drag(cx), slot).is_some() {
                        this.set_hover_swap(slot, cx);
                        return;
                    }
                    let middle = event.bounds.center().x;
                    let gap = slot + usize::from(event.event.position.x > middle);
                    this.set_hover_gap(gap, cx);
                },
            ))
            .on_drop(cx.listener(move |this, drag: &BoardDrag, _, cx| {
                if this.hover_swap == Some(slot)
                    && let Some(capture) = this.swappable(drag, slot)
                {
                    this.hover_swap = None;
                    this.gear_picker = None;
                    return this.swap_capture(slot, &capture, cx);
                }
                let gap = this.hover_gap.unwrap_or(slot);
                this.drop_on_board(drag, gap, cx);
            }))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.selected = Some(slot);
                if this
                    .gear_picker
                    .is_some_and(|picker| matches!(picker.at, PickAt::Replace(_)))
                {
                    this.open_gear(PickAt::Replace(slot), cx);
                }
                this.focus.focus(window, cx);
                cx.notify();
            }))
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .child(card)
            .child(h_flex().h(px(ACTIONS_HEIGHT)).gap_1().items_center().when(
                show_actions,
                |actions| {
                    actions
                        .child(
                            Button::new(("replace", slot))
                                .xsmall()
                                .ghost()
                                .label("Replace")
                                .tooltip("Choose other gear for this spot")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.open_gear(PickAt::Replace(slot), cx);
                                })),
                        )
                        .child(
                            Button::new(("remove", slot))
                                .xsmall()
                                .ghost()
                                .label("Remove")
                                .tooltip("Take it off the chain (Delete). Ctrl+Z brings it back")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.remove_block(slot, cx);
                                })),
                        )
                },
            ))
    }

    fn render_jack(&self, label: &'static str) -> impl IntoElement {
        v_flex()
            .flex_shrink_0()
            .h(px(CARD_HEIGHT))
            .w(px(56.))
            .justify_center()
            .items_center()
            .gap_1()
            .child(
                div()
                    .size(px(14.))
                    .rounded_full()
                    .border_2()
                    .border_color(moss())
                    .bg(hsla(0.0, 0.0, 0.0, 0.8))
                    .shadow(vec![glow(0.4, 10.0)]),
            )
            .child(div().text_xs().text_color(fog()).child(label))
    }

    /// The whole chain in signal order, with its own controls beside it.
    pub(super) fn render_strip(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let count = self.preset.blocks.len();
        let mut row: Vec<AnyElement> = vec![self.render_jack("Guitar").into_any_element()];
        for slot in 0..count {
            row.push(self.render_gap(slot, window, cx).into_any_element());
            row.push(self.render_card(slot, cx).into_any_element());
        }
        row.push(self.render_gap(count, window, cx).into_any_element());
        row.push(self.render_jack("Speakers").into_any_element());
        let adding = self
            .gear_picker
            .is_some_and(|picker| !matches!(picker.at, PickAt::Replace(_)));
        v_flex()
            .flex_shrink_0()
            .h(px(STRIP_HEIGHT))
            .border_b_1()
            .border_color(hairline())
            .bg(pane())
            .child(
                h_flex()
                    .px_4()
                    .pt_3()
                    .gap_3()
                    .items_center()
                    .child(div().text_sm().font_semibold().child("Signal chain"))
                    .child(div().text_xs().text_color(fog()).child(match count {
                        0 => "Empty. Add a block to start.".to_owned(),
                        1 => "1 block".to_owned(),
                        count => format!("{count} blocks, played left to right"),
                    }))
                    .child(div().flex_1())
                    .child(
                        Button::new("add-block")
                            .small()
                            .when(adding, |button| button.primary())
                            .when(!adding, |button| button.outline())
                            .label("+ Add block")
                            .tooltip("Pick gear to add (G). Or press + between two blocks")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this
                                    .gear_picker
                                    .is_some_and(|picker| !matches!(picker.at, PickAt::Replace(_)))
                                {
                                    this.close_gear(cx);
                                } else {
                                    let at = this
                                        .selected
                                        .map_or(PickAt::Usual, |slot| PickAt::Gap(slot + 1));
                                    this.open_gear(at, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("arrange")
                            .small()
                            .ghost()
                            .label("Sort")
                            .tooltip("Put the blocks in the usual order")
                            .on_click(cx.listener(|this, _, _, cx| this.arrange(cx))),
                    ),
            )
            .child(
                h_flex()
                    .id("chain")
                    .flex_1()
                    .min_h_0()
                    .px_4()
                    .items_start()
                    .pt_4()
                    .overflow_x_scroll()
                    .children(row),
            )
    }

    /// What the picker is about to do, in words.
    fn picker_title(&self, at: PickAt) -> (String, String) {
        match at {
            PickAt::Usual => (
                "Add a block".into(),
                "It goes where that kind of gear usually sits. Drag it onto the chain to choose \
                 the spot yourself."
                    .into(),
            ),
            PickAt::Gap(gap) => {
                let count = self.preset.blocks.len();
                let before = gap.checked_sub(1).map(|slot| self.short_label(slot));
                let after = (gap < count).then(|| self.short_label(gap));
                (
                    format!("Add block {}", gap + 1),
                    gap_place(before.as_deref(), after.as_deref()),
                )
            }
            PickAt::Replace(slot) => (
                format!("Replace block {}", slot + 1),
                format!(
                    "In place of {}. A capture of the same kind keeps its knob settings.",
                    self.short_label(slot)
                ),
            ),
        }
    }

    fn render_option(
        &self,
        id: usize,
        gear: Gear,
        at: PickAt,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let art = gear.art();
        let current = match (at, &gear) {
            (PickAt::Replace(slot), Gear::Capture(capture)) => {
                self.preset.blocks[slot].asset.as_ref() == Some(&capture.asset)
            }
            (PickAt::Replace(slot), Gear::BuiltIn(kind)) => {
                let block = &self.preset.blocks[slot];
                block.kind == *kind && block.asset.is_none()
            }
            _ => false,
        };
        let keeps = match (at, &gear) {
            (PickAt::Replace(slot), Gear::Capture(capture)) => {
                !current && self.preset.blocks[slot].kind == capture_block_kind(capture.kind)
            }
            _ => false,
        };
        let source = match &gear {
            Gear::Capture(capture) => capture.origin.as_ref().map_or_else(
                || "Your capture".to_owned(),
                |origin| format!("by {}", origin.creator),
            ),
            Gear::BuiltIn(_) => "Built in".to_owned(),
        };
        let drag = BoardDrag {
            source: DragSource::Rack(gear.clone()),
            art: art.clone(),
        };
        v_flex()
            .id(("option", id))
            .relative()
            .flex_shrink_0()
            .w(px(OPTION_WIDTH))
            .px_2()
            .pb_2()
            .pt(px(8.0 + STRIPE))
            .gap_1()
            .rounded_lg()
            .overflow_hidden()
            .border_1()
            .border_color(if current { moss() } else { hairline() })
            .bg(if current { moss_glow(0.08) } else { well() })
            .when(!current, |option| {
                option
                    .cursor_pointer()
                    .hover(|option| option.border_color(moss()).bg(moss_glow(0.06)))
                    .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
                    .on_click(cx.listener(move |this, _, _, cx| this.pick(&gear, cx)))
            })
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(STRIPE))
                    .bg(art.shape.tint().opacity(0.9)),
            )
            .child(
                div()
                    .text_xs()
                    .font_semibold()
                    .truncate()
                    .text_color(art.shape.tint())
                    .child(art.shape.role().to_uppercase()),
            )
            .child(
                div()
                    .h(px(OPTION_ART.1))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(art::thumb_fit(&art, true, OPTION_ART)),
            )
            .child(div().text_sm().truncate().child(art.title.clone()))
            .child(
                div()
                    .text_xs()
                    .text_color(fog())
                    .truncate()
                    .child(if current {
                        "In this spot now".to_owned()
                    } else if keeps {
                        "Keeps the knobs".to_owned()
                    } else {
                        source
                    }),
            )
    }

    /// The one panel for choosing gear, opened from the chain and pointed at
    /// where the choice will go.
    pub(super) fn render_picker(&self, picker: Picker, cx: &mut Context<Self>) -> impl IntoElement {
        let (title, place) = self.picker_title(picker.at);
        let captures = self
            .catalog
            .iter()
            .cloned()
            .map(|capture| Gear::Capture(Box::new(capture)));
        let built_in = BUILT_IN.iter().map(|kind| Gear::BuiltIn(*kind));
        let gear: Vec<(Gear, GearArt)> = captures
            .chain(built_in)
            .map(|gear| {
                let art = gear.art();
                (gear, art)
            })
            .filter(|(gear, art)| picker.shelf.holds(gear, art))
            .collect();
        let empty = gear.is_empty();
        let options: Vec<AnyElement> = gear
            .into_iter()
            .enumerate()
            .map(|(id, (gear, _))| {
                self.render_option(id, gear, picker.at, cx)
                    .into_any_element()
            })
            .collect();
        let shelves: Vec<AnyElement> = Shelf::ALL
            .iter()
            .enumerate()
            .map(|(index, (shelf, name))| {
                let shelf = *shelf;
                pill(("shelf", index), *name, picker.shelf == shelf)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_shelf(shelf, cx)))
                    .into_any_element()
            })
            .collect();
        let target = match picker.at {
            PickAt::Replace(slot) => Some(slot),
            _ => None,
        };
        v_flex()
            .id("picker")
            .flex_shrink_0()
            .h(px(PICKER_HEIGHT))
            .border_b_1()
            .border_color(moss_glow(0.4))
            .bg(sheet().opacity(1.0))
            .child(
                h_flex()
                    .px_4()
                    .pt_3()
                    .gap_4()
                    .items_center()
                    .child(
                        v_flex()
                            .flex_shrink_0()
                            .max_w(px(460.))
                            .child(div().font_semibold().text_color(moss()).child(title))
                            .child(div().text_xs().text_color(fog()).truncate().child(place)),
                    )
                    .child(h_flex().gap_1().items_center().children(shelves))
                    .child(div().flex_1())
                    .child(
                        Button::new("import-nam")
                            .small()
                            .ghost()
                            .label("Import .nam files")
                            .tooltip("Add your own NAM captures to this list")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_captures(window, cx);
                            })),
                    )
                    .child(
                        Button::new("find-tone3000")
                            .small()
                            .ghost()
                            .label("Find on TONE3000")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.gear_picker = None;
                                this.open_tone3000(target, cx);
                            })),
                    )
                    .child(
                        Button::new("close-picker")
                            .small()
                            .ghost()
                            .label("Close")
                            .tooltip("Esc")
                            .on_click(cx.listener(|this, _, _, cx| this.close_gear(cx))),
                    ),
            )
            .child(
                h_flex()
                    .id("picker-gear")
                    .flex_1()
                    .min_h_0()
                    .px_4()
                    .py_3()
                    .gap_3()
                    .items_start()
                    .overflow_x_scroll()
                    .when(empty, |row| {
                        row.child(
                            div().text_sm().text_color(fog()).child(
                                "No captures yet. Import .nam files, or find some on TONE3000.",
                            ),
                        )
                    })
                    .children(options),
            )
    }
}

/// Cards are opaque, so the glow round a selected one stays outside it.
fn card() -> Hsla {
    hsla(0.42, 0.05, 0.085, 1.0)
}

fn card_lit() -> Hsla {
    hsla(0.41, 0.16, 0.11, 1.0)
}

fn ordinal(number: usize) -> String {
    let suffix = match (number % 10, number % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{number}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::{gap_place, ordinal, status};

    #[test]
    fn a_gap_is_named_by_its_neighbours() {
        assert_eq!(
            gap_place(None, None),
            "First block, between the guitar and the speakers"
        );
        assert_eq!(
            gap_place(Some("Cinder (2 · Overdrive)"), None),
            "After Cinder (2 · Overdrive), last before the speakers"
        );
        assert_eq!(gap_place(Some("A"), Some("B")), "Between A and B");
    }

    #[test]
    fn ordinals_read_naturally() {
        assert_eq!(ordinal(1), "1st");
        assert_eq!(ordinal(2), "2nd");
        assert_eq!(ordinal(3), "3rd");
        assert_eq!(ordinal(4), "4th");
        assert_eq!(ordinal(11), "11th");
        assert_eq!(ordinal(22), "22nd");
    }

    #[test]
    fn a_missing_file_outranks_the_switch() {
        assert_eq!(status(true, false), "On");
        assert_eq!(status(false, false), "Off");
        assert_eq!(status(true, true), "File missing");
    }
}
