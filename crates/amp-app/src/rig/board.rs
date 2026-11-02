//! The chain strip and the stage. The strip shows the whole rig in signal
//! order and takes drags from the gear drawer or along itself, with live
//! changes on every drop. The stage shows the selected unit up close.

use super::{
    AMP_QUALITY, FOOTER_HEIGHT, HEADER_HEIGHT, KNOB_TONE_LEVEL, Overlay, Rig, assistant_view,
    explorer::pill,
};
use crate::{
    art::{self, GearArt, STAGE_HEIGHT, Shape},
    knob::{Knob, KnobLook},
    style::{self, bar, fog, glow, hairline, moss, moss_glow, pane},
};
use amp_dsp::{AmpInfo, BlockKind};
use amp_preset::{AssetKind, BlockPreset, CaptureEntry, CaptureKind, MAX_SCENES};
use gpui_kit::{
    base::spring,
    component::{
        ActiveTheme, Disableable, Sizable, StyledExt,
        button::{Button, ButtonVariants},
        h_flex, v_flex,
    },
    prelude::FluentBuilder,
    *,
};

const GAP: f32 = 16.0;
const STRIP_HEIGHT: f32 = 158.0;
const SCENE_BAR_HEIGHT: f32 = 48.0;
const SCENE_LEVEL_KNOB: Pixels = px(30.);
const UNIT_SLOT: f32 = 114.0;
/// How much of a thumbnail pedal is its footswitch.
const FOOTSWITCH_SHARE: f32 = 0.42;
const DETAILS_WIDTH: f32 = 232.0;
const DRAWER_WIDTH: f32 = 232.0;
const STAGE_MARGIN: f32 = 28.0;
const STAGE_SCALE: (f32, f32) = (0.55, 1.2);
const DETAILS_KNOB: Pixels = px(58.);
const BUILT_IN: [BlockKind; 8] = [
    BlockKind::Tube,
    BlockKind::Cab,
    BlockKind::Compressor,
    BlockKind::Drive,
    BlockKind::Gate,
    BlockKind::Chorus,
    BlockKind::Delay,
    BlockKind::Reverb,
];

/// Something in the rack that can go on the board.
#[derive(Clone)]
pub(super) enum Gear {
    BuiltIn(BlockKind),
    Capture(Box<CaptureEntry>),
}

impl Gear {
    fn block(&self) -> BlockPreset {
        match self {
            Self::BuiltIn(kind) => BlockPreset::new(*kind, true),
            Self::Capture(capture) => {
                let mut block = BlockPreset::new(capture_block_kind(capture.kind), true);
                block.asset = Some(capture.asset.clone());
                block.origin = capture.origin.clone();
                block
            }
        }
    }

    fn art(&self) -> GearArt {
        match self {
            Self::BuiltIn(kind) => block_art(&BlockPreset::new(*kind, true), None),
            Self::Capture(capture) => block_art(&self.block(), Some(capture.title.as_str())),
        }
    }
}

/// The block a capture of this kind runs in.
fn capture_block_kind(kind: CaptureKind) -> BlockKind {
    match kind {
        CaptureKind::Amp => BlockKind::Amp,
        CaptureKind::Pedal => BlockKind::Pedal,
    }
}

pub(super) fn block_art(block: &BlockPreset, name: Option<&str>) -> GearArt {
    let asset_name = block.asset.as_ref().map(|asset| {
        asset
            .name
            .rsplit_once('.')
            .map_or(asset.name.as_str(), |(stem, _)| stem)
    });
    let title = block
        .origin
        .as_ref()
        .map(|origin| origin.title.as_str())
        .or(name)
        .or(asset_name)
        .unwrap_or(art::model_name(block.kind))
        .to_owned();
    let shape = Shape::for_kind(block.kind, &title, block.asset.is_some());
    let type_label = shape.type_label().into();
    GearArt {
        shape,
        title: title.into(),
        type_label,
    }
}

#[derive(Clone)]
enum DragSource {
    Rack(Gear),
    Board(usize),
}

/// What is under the pointer during a drag, and its floating preview.
#[derive(Clone)]
pub(super) struct BoardDrag {
    source: DragSource,
    art: GearArt,
}

impl Render for BoardDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // Offset so the gear hangs off the pointer and the slot stays visible.
        art::thumb(&self.art, true)
            .ml(px(10.))
            .mt(px(10.))
            .opacity(0.9)
    }
}

/// The unit a step lands on, wrapping at the ends. With nothing selected, Right
/// starts at the first unit and Left at the last.
fn step_selection(current: Option<usize>, step: isize, count: isize) -> usize {
    let start = current.map_or(if step > 0 { -1 } else { 0 }, |slot| slot as isize);
    (start + step).rem_euclid(count) as usize
}

/// Where a new block goes: ahead of the first unit that usually follows it.
pub(super) fn place_after(rig: &Rig, block: &BlockPreset) -> usize {
    let rank = block_art(block, None).shape.rank();
    (0..rig.preset.blocks.len())
        .find(|slot| rig.shape_of(*slot).rank() > rank)
        .unwrap_or(rig.preset.blocks.len())
}

/// Where `moved` lands once it is taken out of the chain, for a drop in `gap`.
fn landing(moved: usize, gap: usize) -> usize {
    if gap > moved { gap - 1 } else { gap }
}

impl Rig {
    pub(super) fn block_name(&self, slot: usize) -> Option<&str> {
        self.captures.get(slot)?.as_ref().and_then(capture_name)
    }

    fn drop_on_board(&mut self, drag: &BoardDrag, gap: usize, cx: &mut Context<Self>) {
        self.hover_gap = None;
        self.hover_swap = None;
        match &drag.source {
            DragSource::Rack(gear) => self.insert_block(gear.block(), gap, cx),
            DragSource::Board(from) => self.move_block(*from, landing(*from, gap), cx),
        }
    }

    pub(super) fn insert_block(&mut self, block: BlockPreset, slot: usize, cx: &mut Context<Self>) {
        self.remember();
        let slot = slot.min(self.preset.blocks.len());
        self.preset.insert_block(slot, block);
        self.captures.insert(slot, None);
        self.problems.insert(slot, None);
        self.selected = Some(slot);
        self.rebuild_chain(cx);
    }

    fn move_block(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if from == to || to >= self.preset.blocks.len() {
            return;
        }
        self.remember();
        self.preset.move_block(from, to);
        let capture = self.captures.remove(from);
        self.captures.insert(to, capture);
        let problem = self.problems.remove(from);
        self.problems.insert(to, problem);
        self.selected = Some(to);
        self.rebuild_chain(cx);
    }

    fn remove_block(&mut self, slot: usize, cx: &mut Context<Self>) {
        self.remember();
        self.preset.remove_block(slot);
        self.captures.remove(slot);
        self.problems.remove(slot);
        self.selected = None;
        self.rebuild_chain(cx);
    }

    /// Puts another capture in a unit, keeping its place and its settings, in
    /// every scene. This is how a player auditions captures against each other.
    pub(super) fn swap_capture(
        &mut self,
        slot: usize,
        capture: &CaptureEntry,
        cx: &mut Context<Self>,
    ) {
        let Some(block) = self.preset.blocks.get(slot) else {
            return;
        };
        if block.kind != capture_block_kind(capture.kind)
            || block.asset.as_ref() == Some(&capture.asset)
        {
            return;
        }
        self.remember();
        let block = &mut self.preset.blocks[slot];
        block.asset = Some(capture.asset.clone());
        block.origin.clone_from(&capture.origin);
        self.selected = Some(slot);
        self.rebuild_chain(cx);
    }

    /// Double-clicking in the rack places gear where it usually goes.
    fn place_block(&mut self, gear: &Gear, cx: &mut Context<Self>) {
        let block = gear.block();
        self.insert_block(block.clone(), place_after(self, &block), cx);
    }

    fn shape_of(&self, slot: usize) -> Shape {
        block_art(&self.preset.blocks[slot], self.block_name(slot)).shape
    }

    /// Sorts the board into the usual order. A guideline, not a rule, so
    /// anything can be dragged back out of it.
    fn arrange(&mut self, cx: &mut Context<Self>) {
        self.remember();
        let mut order: Vec<usize> = (0..self.preset.blocks.len()).collect();
        order.sort_by_key(|slot| self.shape_of(*slot).rank());
        self.preset.reorder(&order);
        self.captures = order.iter().map(|s| self.captures[*s].clone()).collect();
        self.problems = order.iter().map(|s| self.problems[*s].clone()).collect();
        self.selected = None;
        self.rebuild_chain(cx);
    }

    pub(super) fn remove_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(slot) = self
            .selected
            .filter(|slot| *slot < self.preset.blocks.len())
        {
            self.remove_block(slot, cx);
        }
    }

    pub(super) fn bypass_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(slot) = self
            .selected
            .filter(|slot| *slot < self.preset.blocks.len())
        {
            self.toggle_block(slot, cx);
        }
    }

    /// Puts something on the stage if nothing is there yet.
    pub(super) fn open_selected(&mut self, cx: &mut Context<Self>) {
        if self.selected.is_none() && !self.preset.blocks.is_empty() {
            self.selected = Some(0);
        }
        cx.notify();
    }

    pub(super) fn close_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_sign_in();
        self.overlay = Overlay::None;
        self.focus.focus(window, cx);
        cx.notify();
    }

    /// Escape backs out one level: first any open panel, then the selection.
    pub(super) fn close_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay == Overlay::None {
            self.selected = None;
            cx.notify();
        } else {
            self.close_overlay(window, cx);
        }
    }

    pub(super) fn toggle_gear(&mut self, cx: &mut Context<Self>) {
        self.gear_open = !self.gear_open;
        cx.notify();
    }

    pub(super) fn select_step(&mut self, step: isize, cx: &mut Context<Self>) {
        let count = self.preset.blocks.len() as isize;
        if count > 0 {
            self.selected = Some(step_selection(self.selected, step, count));
            cx.notify();
        }
    }

    pub(super) fn move_selected(&mut self, step: isize, cx: &mut Context<Self>) {
        let Some(slot) = self.selected else {
            return;
        };
        let target = slot as isize + step;
        if target >= 0 {
            self.move_block(slot, target as usize, cx);
        }
    }

    fn render_gap(
        &self,
        gap: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let hovered = cx.has_active_drag() && self.hover_gap == Some(gap);
        let width = spring(
            format!("board-gap-{gap}"),
            if hovered { 84.0_f32 } else { 0.0 },
            cx.theme().motion_tokens().spring_control,
            window,
            cx,
        );
        let cable = 0.22 + (self.output_level * 3.0).min(0.7);
        h_flex()
            .id(("gap", gap))
            .h(px(UNIT_SLOT))
            .flex_shrink_0()
            .w(px(GAP + width))
            .items_center()
            .justify_center()
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
            .child(if hovered {
                div()
                    .h(px(UNIT_SLOT - 8.))
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

    /// The capture in a drag from the drawer, if it can swap into `slot`.
    fn swappable(&self, drag: &BoardDrag, slot: usize) -> Option<CaptureEntry> {
        let DragSource::Rack(Gear::Capture(capture)) = &drag.source else {
            return None;
        };
        let block = self.preset.blocks.get(slot)?;
        (block.kind == capture_block_kind(capture.kind)).then(|| (**capture).clone())
    }

    fn render_unit(&self, slot: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let block = &self.preset.blocks[slot];
        let art = block_art(block, self.block_name(slot));
        let selected = self.selected == Some(slot);
        let enabled = block.enabled;
        let is_stomp = art.shape.is_stomp();
        let badge = block.origin.as_ref().map(|origin| self.tone_badge(origin));
        let swap_target = self.hover_swap == Some(slot) && cx.has_active_drag();
        let has_problem = self.problems.get(slot).is_some_and(Option::is_some);
        let warning = cx.theme().warning;
        let drag = BoardDrag {
            source: DragSource::Board(slot),
            art: art.clone(),
        };
        v_flex()
            .id(("unit", slot))
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .cursor_grab()
            .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
            .on_drag_move::<BoardDrag>(cx.listener(
                move |this, event: &DragMoveEvent<BoardDrag>, _, cx| {
                    if !event.bounds.contains(&event.event.position) {
                        return;
                    }
                    // A capture held over a unit of its own kind swaps into it.
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
                    return this.swap_capture(slot, &capture, cx);
                }
                let gap = this.hover_gap.unwrap_or(slot);
                this.drop_on_board(drag, gap, cx);
            }))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.selected = Some(slot);
                this.focus.focus(window, cx);
                cx.notify();
            }))
            .child(
                div().h(px(UNIT_SLOT)).flex().items_center().child(
                    art::thumb(&art, enabled)
                        .when(is_stomp, |unit| {
                            // The footswitch works like a real one: press it to bypass.
                            unit.child(
                                div()
                                    .id(("footswitch", slot))
                                    .absolute()
                                    .bottom_0()
                                    .left_0()
                                    .right_0()
                                    .h(relative(FOOTSWITCH_SHARE))
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.toggle_block(slot, cx);
                                    })),
                            )
                        })
                        .children(badge)
                        .when(swap_target, |unit| {
                            unit.rounded_lg()
                                .border_2()
                                .border_color(moss())
                                .shadow(vec![glow(0.6, 12.0)])
                        })
                        .when(has_problem, |unit| {
                            // A file this unit needs is missing, so it's passing
                            // the signal through. The panel says which.
                            unit.child(
                                div()
                                    .absolute()
                                    .top(px(4.))
                                    .right(px(4.))
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
                ),
            )
            .child(
                div()
                    .max_w(px(112.))
                    .px_1()
                    .rounded_sm()
                    .text_xs()
                    .truncate()
                    .text_color(if selected { moss() } else { fog() })
                    .child(art.title.clone()),
            )
            .child(
                div()
                    .w(px(36.))
                    .h(px(2.))
                    .rounded_sm()
                    .when(selected, |mark| {
                        mark.bg(moss()).shadow(vec![glow(0.6, 8.0)])
                    }),
            )
    }

    fn render_jack(&self, label: &'static str) -> impl IntoElement {
        v_flex()
            .flex_shrink_0()
            .h(px(UNIT_SLOT))
            .w(px(58.))
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

    fn render_strip(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut row: Vec<AnyElement> = vec![self.render_jack("Guitar").into_any_element()];
        for slot in 0..self.preset.blocks.len() {
            row.push(self.render_gap(slot, window, cx).into_any_element());
            row.push(self.render_unit(slot, cx).into_any_element());
        }
        row.push(
            self.render_gap(self.preset.blocks.len(), window, cx)
                .into_any_element(),
        );
        row.push(self.render_jack("Speakers").into_any_element());
        h_flex()
            .flex_shrink_0()
            .h(px(STRIP_HEIGHT))
            .gap_3()
            .items_center()
            .px_3()
            .border_b_1()
            .border_color(hairline())
            .bg(pane())
            .child(
                h_flex()
                    .id("chain")
                    .flex_1()
                    .min_w_0()
                    .items_start()
                    .overflow_x_scroll()
                    .children(row),
            )
            .child(
                v_flex()
                    .flex_shrink_0()
                    .gap_1()
                    .child(
                        Button::new("browse-tones")
                            .small()
                            .ghost()
                            .label("Browse tones")
                            .tooltip("Find a tone on TONE3000")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_tone3000(None, cx);
                            })),
                    )
                    .child(
                        Button::new("add-gear")
                            .small()
                            .when(self.gear_open, |button| button.outline())
                            .when(!self.gear_open, |button| button.ghost())
                            .label("Add gear")
                            .tooltip("Press G")
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_gear(cx))),
                    )
                    .child(
                        Button::new("arrange")
                            .small()
                            .ghost()
                            .label("Arrange")
                            .tooltip("Sort into the usual order")
                            .on_click(cx.listener(|this, _, _, cx| this.arrange(cx))),
                    ),
            )
    }

    /// The tone's scenes and its Level. A scene keeps every switch and knob of
    /// the same chain, so moving between them is instant and tails ring on.
    fn render_scene_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let scenes = self.preset.scenes.len();
        let playing = self.preset.scene;
        let pills: Vec<AnyElement> = (0..scenes)
            .map(|index| {
                let name = &self.preset.scenes[index].name;
                let title = if name.is_empty() {
                    format!("Scene {}", index + 1)
                } else {
                    name.clone()
                };
                pill(("scene", index), title, index == playing)
                    .on_click(cx.listener(move |this, _, _, cx| this.select_scene(index, cx)))
                    .into_any_element()
            })
            .collect();
        let level = Knob::new(
            "tone-level",
            KNOB_TONE_LEVEL,
            self.preset.level(),
            SCENE_LEVEL_KNOB,
            cx.listener(|this, value: &f32, _, cx| this.set_tone_level(*value, cx)),
        );
        let level_label = if scenes > 0 {
            "Scene level"
        } else {
            "Tone level"
        };
        h_flex()
            .flex_shrink_0()
            .h(px(SCENE_BAR_HEIGHT))
            .px_3()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(hairline())
            .bg(pane())
            .child(div().text_xs().text_color(fog()).child("Scenes"))
            .when(scenes == 0, |bar| {
                bar.child(
                    div()
                        .text_xs()
                        .text_color(fog())
                        .child("One sound. Add a scene for another setting of this chain."),
                )
            })
            .children(pills)
            .when(scenes < MAX_SCENES, |bar| {
                bar.child(
                    Button::new("add-scene")
                        .small()
                        .ghost()
                        .label(if scenes == 0 { "Add scene" } else { "+" })
                        .tooltip("Copies this sound. Keys 1 to 4 switch scenes")
                        .on_click(cx.listener(|this, _, _, cx| this.add_scene(cx))),
                )
            })
            .when(scenes > 0, |bar| {
                bar.child(
                    Button::new("remove-scene")
                        .small()
                        .ghost()
                        .label("Remove scene")
                        .on_click(cx.listener(|this, _, _, cx| this.remove_scene(cx))),
                )
            })
            .child(div().flex_1())
            .child(div().text_xs().text_color(fog()).child(level_label))
            .child(level)
            .child(
                Button::new("match-level")
                    .small()
                    .ghost()
                    .label(if self.matching_level {
                        "Matching"
                    } else {
                        "Match level"
                    })
                    .disabled(self.matching_level)
                    .tooltip("Sets the level to sit with the factory tones")
                    .on_click(cx.listener(|this, _, _, cx| this.match_level(cx))),
            )
    }

    fn render_details(&self, slot: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let block = &self.preset.blocks[slot];
        let kind = block.kind;
        let art = block_art(block, self.block_name(slot));
        let info = self.captures.get(slot).and_then(Option::as_ref);
        let tiers = info.map_or(1, |info| info.quality_tiers);
        let detail = info.map(|info| {
            let kind = if tiers > 1 {
                format!("NAM A2, {tiers} quality sizes")
            } else {
                format!("NAM {}", info.architecture)
            };
            [Some(kind), info.gear.clone(), info.tone_type.clone()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("  /  ")
        });
        let doubled_cab = kind == BlockKind::Cab
            && self
                .captures
                .iter()
                .flatten()
                .any(|info| info.includes_cab == Some(true));
        let is_cab = matches!(art.shape, Shape::Cabinet { .. });
        let takes_tone = matches!(
            kind,
            BlockKind::Amp | BlockKind::Pedal | BlockKind::Cab | BlockKind::Tube
        );
        let problem = self.problems.get(slot).cloned().flatten();
        let swaps = self.render_swaps(slot, cx);
        let credit = block
            .origin
            .as_ref()
            .map(|origin| self.render_tone_credit(origin, cx).into_any_element());
        let browse = takes_tone.then(|| {
            Button::new("browse-block-tones")
                .small()
                .outline()
                .label("Browse tones")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.open_tone3000(Some(slot), cx);
                }))
        });
        let last = self.preset.blocks.len() - 1;
        let move_button = |id, label, to: usize, disabled: bool, cx: &mut Context<Self>| {
            Button::new(id)
                .small()
                .ghost()
                .label(label)
                .when(!disabled, |button| {
                    button.on_click(cx.listener(move |this, _, _, cx| {
                        this.move_block(slot, to, cx);
                    }))
                })
        };
        v_flex()
            .id("details")
            .flex_shrink_0()
            .w(px(DETAILS_WIDTH))
            .h_full()
            .gap_3()
            .p_3()
            .overflow_y_scroll()
            .border_l_1()
            .border_color(hairline())
            .bg(pane())
            .child(
                v_flex()
                    .min_w_0()
                    .child(
                        div()
                            .font_family(art::PRINT_FONT)
                            .text_lg()
                            .font_semibold()
                            .child(art.title.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(fog())
                            .child(art.type_label.clone()),
                    )
                    .when_some(detail, |column, detail| {
                        column.child(div().mt_1().text_xs().text_color(fog()).child(detail))
                    }),
            )
            .when(is_cab, |panel| {
                panel.child(
                    h_flex()
                        .justify_center()
                        .child(self.knob(slot, 0, DETAILS_KNOB, cx)),
                )
            })
            .when_some(problem, |panel, problem| {
                panel.child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().warning)
                                .child(problem),
                        )
                        .child(div().text_xs().text_color(fog()).child(if is_cab {
                            "Playing the built-in cabinet until it's found."
                        } else {
                            "Passing the signal through until it's found."
                        }))
                        .when(kind != BlockKind::Cab, |column| {
                            column.child(
                                Button::new("locate-capture")
                                    .small()
                                    .outline()
                                    .label("Locate file")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.locate_capture(slot, window, cx);
                                    })),
                            )
                        }),
                )
            })
            .when(doubled_cab, |panel| {
                panel.child(div().text_sm().text_color(cx.theme().warning).child(
                    "One of your captures already includes a cabinet, so you may be hearing two.",
                ))
            })
            .children(credit)
            .children(swaps)
            .child(
                v_flex()
                    .gap_1()
                    .children(browse)
                    .when(
                        AssetKind::for_block(kind) == Some(AssetKind::ImpulseResponse),
                        |column| {
                            column.child(
                                Button::new("load-ir")
                                    .small()
                                    .outline()
                                    .label("Load cab IR")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.load_cab_ir(slot, window, cx);
                                    })),
                            )
                        },
                    )
                    .child(
                        Button::new("bypass")
                            .small()
                            .outline()
                            .label(if block.enabled { "Bypass" } else { "Turn on" })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_block(slot, cx);
                            })),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(move_button(
                                "move-left",
                                "Move left",
                                slot.saturating_sub(1),
                                slot == 0,
                                cx,
                            ))
                            .child(move_button(
                                "move-right",
                                "Move right",
                                slot + 1,
                                slot == last,
                                cx,
                            )),
                    )
                    .child(
                        Button::new("remove")
                            .small()
                            .ghost()
                            .label("Remove")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.remove_block(slot, cx);
                            })),
                    ),
            )
    }

    /// The other captures of this unit's kind, to swap in with one click. The
    /// unit keeps its place and its settings, so they compare fairly.
    fn render_swaps(&self, slot: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let block = &self.preset.blocks[slot];
        let captures: Vec<CaptureEntry> = self
            .catalog
            .iter()
            .filter(|capture| capture_block_kind(capture.kind) == block.kind)
            .cloned()
            .collect();
        // Worth a list only when there's something other than what's loaded.
        let others = captures
            .iter()
            .filter(|capture| block.asset.as_ref() != Some(&capture.asset))
            .count();
        if others == 0 {
            return None;
        }
        let rows: Vec<AnyElement> = captures
            .into_iter()
            .enumerate()
            .map(|(index, capture)| {
                let current = block.asset.as_ref() == Some(&capture.asset);
                div()
                    .id(("swap", index))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_sm()
                    .truncate()
                    .cursor_pointer()
                    .when(current, |row| row.text_color(moss()).bg(moss_glow(0.08)))
                    .when(!current, |row| row.hover(|row| row.bg(moss_glow(0.06))))
                    .child(capture.title.clone())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.swap_capture(slot, &capture, cx);
                    }))
                    .into_any_element()
            })
            .collect();
        Some(
            v_flex()
                .gap_px()
                .child(
                    div()
                        .text_xs()
                        .text_color(fog())
                        .child("Swap capture, keeping these settings"),
                )
                .children(rows)
                .into_any_element(),
        )
    }

    /// Fits the unit to the room left in the window, so nothing clips on a
    /// small one.
    fn stage_scale(&self, shape: &Shape, window: &Window) -> f32 {
        let viewport = window.viewport_size();
        let room_high = f32::from(viewport.height)
            - HEADER_HEIGHT
            - FOOTER_HEIGHT
            - STRIP_HEIGHT
            - SCENE_BAR_HEIGHT
            - STAGE_MARGIN;
        let drawer = if self.gear_open { DRAWER_WIDTH } else { 0.0 };
        let panel = if self.assistant.open {
            assistant_view::PANEL_WIDTH
        } else {
            0.0
        };
        let room_wide = f32::from(viewport.width) - DETAILS_WIDTH - drawer - panel - STAGE_MARGIN;
        let (width, height) = shape.stage_size();
        (room_high / height.max(STAGE_HEIGHT * 0.78))
            .min(room_wide / width)
            .clamp(STAGE_SCALE.0, STAGE_SCALE.1)
    }

    fn render_stage_unit(
        &self,
        slot: usize,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let block = &self.preset.blocks[slot];
        let kind = block.kind;
        let art = block_art(block, self.block_name(slot));
        let scale = self.stage_scale(&art.shape, window);
        let tiers = self
            .captures
            .get(slot)
            .and_then(Option::as_ref)
            .map_or(1, |info| info.quality_tiers);
        let params: Vec<usize> = (0..kind.params().len())
            .filter(|param| !kind.uses_asset() || *param != AMP_QUALITY || tiers > 1)
            .collect();
        let look = if art.shape.is_stomp() {
            KnobLook::Pedal
        } else {
            KnobLook::Amp
        };
        let size = px(art::stage_knob(&art.shape, params.len()) * scale);
        let ink = art::faceplate_ink(&art.shape);
        let knobs: Vec<AnyElement> = params
            .iter()
            .map(|param| {
                self.knob(slot, *param, size, cx)
                    .on_faceplate(look, ink)
                    .into_any_element()
            })
            .collect();
        let area = art::PEDAL_SWITCH_AREA;
        let label = block
            .origin
            .as_ref()
            .and_then(|origin| self.tone_image(origin));
        art::stage(&art, block.enabled, scale, knobs, label).when(art.shape.is_stomp(), |unit| {
            unit.child(
                div()
                    .id("stage-footswitch")
                    .absolute()
                    .left(relative(area.0))
                    .top(relative(area.1))
                    .w(relative(area.2))
                    .h(relative(area.3))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_block(slot, cx))),
            )
        })
    }

    pub(super) fn render_board_view(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self
            .selected
            .filter(|slot| *slot < self.preset.blocks.len());
        let empty = if self.preset.blocks.is_empty() {
            "Press Add gear, then drag a unit onto the chain."
        } else {
            "Select a unit in the chain to work on it."
        };
        v_flex()
            .size_full()
            .child(self.render_strip(window, cx))
            .child(self.render_scene_bar(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .justify_center()
                            .items_center()
                            .overflow_hidden()
                            .map(|stage| match selected {
                                Some(slot) => stage.child(self.render_stage_unit(slot, window, cx)),
                                None => stage.child(div().text_color(fog()).child(empty)),
                            }),
                    )
                    .when_some(selected, |row, slot| {
                        row.child(self.render_details(slot, cx))
                    })
                    .when(self.gear_open, |row| row.child(self.render_rack(cx))),
            )
    }

    fn render_rack_item(&self, id: usize, gear: Gear, cx: &mut Context<Self>) -> impl IntoElement {
        let art = gear.art();
        let credit = match &gear {
            Gear::Capture(capture) => capture
                .origin
                .as_ref()
                .map(|origin| format!("{}, by {}", origin.model, origin.creator)),
            Gear::BuiltIn(_) => None,
        };
        let drag = BoardDrag {
            source: DragSource::Rack(gear.clone()),
            art: art.clone(),
        };
        v_flex()
            .id(("rack", id))
            .px_2()
            .py_1()
            .rounded_md()
            .cursor_grab()
            .hover(|item| item.bg(moss_glow(0.08)))
            .child(div().text_sm().truncate().child(art.title.clone()))
            .child(
                div()
                    .text_xs()
                    .text_color(fog())
                    .child(art.type_label.clone()),
            )
            .when_some(credit, |item, credit| {
                item.child(div().text_xs().text_color(fog()).truncate().child(credit))
            })
            .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if event.click_count() >= 2 {
                    this.place_block(&gear, cx);
                }
            }))
    }

    pub(super) fn render_rack(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let group = |title: &'static str| div().mt_2().text_xs().text_color(fog()).child(title);
        let of_kind = |kind: CaptureKind| {
            self.catalog
                .iter()
                .filter(move |capture| capture.kind == kind)
                .cloned()
                .map(|capture| Gear::Capture(Box::new(capture)))
                .collect::<Vec<_>>()
        };
        let amps = of_kind(CaptureKind::Amp);
        let pedals = of_kind(CaptureKind::Pedal);
        let mut id = 0;
        let mut items = |gear: Vec<Gear>, cx: &mut Context<Self>| -> Vec<AnyElement> {
            gear.into_iter()
                .map(|gear| {
                    id += 1;
                    self.render_rack_item(id, gear, cx).into_any_element()
                })
                .collect()
        };
        let amp_items = items(amps, cx);
        let pedal_items = items(pedals, cx);
        let built_in = items(
            BUILT_IN.iter().map(|kind| Gear::BuiltIn(*kind)).collect(),
            cx,
        );
        let empty = self.catalog.is_empty();
        v_flex()
            .id("rack")
            .flex_shrink_0()
            .w(px(DRAWER_WIDTH))
            .h_full()
            .overflow_y_scroll()
            .gap_px()
            .p_3()
            .border_l_1()
            .border_color(hairline())
            .bg(bar())
            .on_drop(cx.listener(|this, drag: &BoardDrag, _, cx| {
                // Dragging a unit back to the drawer takes it off the chain.
                if let DragSource::Board(slot) = drag.source {
                    this.remove_block(slot, cx);
                }
            }))
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(div().font_semibold().child("Gear"))
                    .child(
                        Button::new("add-capture")
                            .small()
                            .outline()
                            .label("Add .nam")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_captures(window, cx);
                            })),
                    ),
            )
            .when(empty, |rack| {
                rack.child(
                    div()
                        .mt_2()
                        .text_xs()
                        .text_color(fog())
                        .child("Add your own NAM captures here, then drag them onto the chain."),
                )
            })
            .when(!amp_items.is_empty(), |rack| {
                rack.child(group("Amp captures"))
            })
            .children(amp_items)
            .when(!pedal_items.is_empty(), |rack| {
                rack.child(group("Pedal captures"))
            })
            .children(pedal_items)
            .child(group("Built in"))
            .children(built_in)
    }

    pub(super) fn render_backdrop(&self) -> impl IntoElement {
        style::backdrop(self.smoke_phase)
    }
}

fn capture_name(info: &AmpInfo) -> Option<&str> {
    info.name.as_deref().or(info.gear.as_deref())
}

#[cfg(test)]
mod tests {
    use super::{landing, step_selection};

    #[test]
    fn arrow_keys_start_at_the_near_end_and_wrap() {
        assert_eq!(step_selection(None, 1, 4), 0);
        assert_eq!(step_selection(None, -1, 4), 3);
        assert_eq!(step_selection(Some(3), 1, 4), 0);
        assert_eq!(step_selection(Some(0), -1, 4), 3);
        assert_eq!(step_selection(Some(1), 1, 4), 2);
    }

    #[test]
    fn a_drop_past_the_unit_accounts_for_it_leaving_its_slot() {
        assert_eq!(landing(1, 3), 2);
        assert_eq!(landing(2, 0), 0);
        assert_eq!(landing(1, 1), 1);
        assert_eq!(landing(1, 2), 1);
    }
}
