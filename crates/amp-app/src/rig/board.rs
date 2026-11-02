//! Block edits and the stage. The chain editor in `chain.rs` is where blocks
//! are added, replaced, removed, and moved; this file holds those edits and
//! shows the selected block up close, with its knobs and what it is.

use super::{
    AMP_QUALITY, FOOTER_HEIGHT, HEADER_HEIGHT, KNOB_TONE_LEVEL, Overlay, Rig, assistant_view,
    chain::{PickAt, STRIP_HEIGHT},
    explorer::pill,
};
use crate::{
    art::{self, GearArt, STAGE_HEIGHT, Shape},
    knob::{Knob, KnobLook},
    style::{self, fog, hairline, moss, pane},
};
use amp_dsp::{AmpInfo, BlockKind};
use amp_preset::{AssetKind, BlockPreset, CaptureEntry, CaptureKind, MAX_SCENES};
use gpui_kit::{
    component::{
        ActiveTheme, Disableable, Sizable, StyledExt,
        button::{Button, ButtonVariants},
        h_flex, v_flex,
    },
    prelude::FluentBuilder,
    *,
};

const SCENE_BAR_HEIGHT: f32 = 48.0;
const SCENE_LEVEL_KNOB: Pixels = px(30.);
const DETAILS_WIDTH: f32 = 248.0;
const STAGE_CAPTION: f32 = 40.0;
const STAGE_MARGIN: f32 = 28.0;
const STAGE_SCALE: (f32, f32) = (0.55, 1.2);
const DETAILS_KNOB: Pixels = px(58.);

/// Something in the rack that can go on the board.
#[derive(Clone)]
pub(super) enum Gear {
    BuiltIn(BlockKind),
    Capture(Box<CaptureEntry>),
}

impl Gear {
    pub(super) fn block(&self) -> BlockPreset {
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

    pub(super) fn art(&self) -> GearArt {
        match self {
            Self::BuiltIn(kind) => block_art(&BlockPreset::new(*kind, true), None),
            Self::Capture(capture) => block_art(&self.block(), Some(capture.title.as_str())),
        }
    }
}

/// The block a capture of this kind runs in.
pub(super) fn capture_block_kind(kind: CaptureKind) -> BlockKind {
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
pub(super) enum DragSource {
    Rack(Gear),
    Board(usize),
}

/// What is under the pointer during a drag, and its floating preview.
#[derive(Clone)]
pub(super) struct BoardDrag {
    pub(super) source: DragSource,
    pub(super) art: GearArt,
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
pub(super) fn landing(moved: usize, gap: usize) -> usize {
    if gap > moved { gap - 1 } else { gap }
}

impl Rig {
    pub(super) fn block_name(&self, slot: usize) -> Option<&str> {
        self.captures.get(slot)?.as_ref().and_then(capture_name)
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

    pub(super) fn move_block(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
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

    pub(super) fn remove_block(&mut self, slot: usize, cx: &mut Context<Self>) {
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

    fn shape_of(&self, slot: usize) -> Shape {
        block_art(&self.preset.blocks[slot], self.block_name(slot)).shape
    }

    /// Sorts the board into the usual order. A guideline, not a rule, so
    /// anything can be dragged back out of it.
    pub(super) fn arrange(&mut self, cx: &mut Context<Self>) {
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

    /// Escape backs out one level: first any open panel, then the gear
    /// picker, then the selection.
    pub(super) fn close_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay == Overlay::None {
            if self.gear_picker.take().is_none() {
                self.selected = None;
            }
            cx.notify();
        } else {
            self.close_overlay(window, cx);
        }
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

    /// What the selected block is, and what it needs. Adding, replacing, and
    /// removing all happen on the chain itself, so this panel only says what
    /// the block is and holds the settings that aren't knobs.
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
        let source = match (&block.origin, &block.asset) {
            (Some(_), _) => "From TONE3000",
            (None, Some(_)) => "Your capture",
            (None, None) => "Built in",
        };
        let doubled_cab = kind == BlockKind::Cab
            && self
                .captures
                .iter()
                .flatten()
                .any(|info| info.includes_cab == Some(true));
        let is_cab = matches!(art.shape, Shape::Cabinet { .. });
        let problem = self.problems.get(slot).cloned().flatten();
        let credit = block
            .origin
            .as_ref()
            .map(|origin| self.render_tone_credit(origin, cx).into_any_element());
        let count = self.preset.blocks.len();
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
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(fog())
                            .child(format!("Block {} of {count}", slot + 1)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_semibold()
                            .text_color(moss())
                            .child(art.shape.role().to_uppercase()),
                    )
                    .child(
                        div()
                            .font_family(art::PRINT_FONT)
                            .text_lg()
                            .font_semibold()
                            .child(art.title.clone()),
                    )
                    .child(div().text_xs().text_color(fog()).child(format!(
                        "{}, {}",
                        art.type_label,
                        source.to_lowercase()
                    )))
                    .when_some(detail, |column, detail| {
                        column.child(div().text_xs().text_color(fog()).child(detail))
                    }),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("bypass")
                            .small()
                            .when(block.enabled, |button| button.outline())
                            .when(!block.enabled, |button| button.primary())
                            .label(if block.enabled { "Turn off" } else { "Turn on" })
                            .tooltip("B")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_block(slot, cx);
                            })),
                    )
                    .child(
                        Button::new("replace-selected")
                            .small()
                            .ghost()
                            .label("Replace")
                            .tooltip("Choose other gear for this spot")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_gear(PickAt::Replace(slot), cx);
                            })),
                    ),
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
            .when(
                AssetKind::for_block(kind) == Some(AssetKind::ImpulseResponse),
                |panel| {
                    panel.child(
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
            .children(credit)
    }

    /// Names the block on the stage, with steps to its neighbours, so the
    /// knobs below always say whose they are.
    fn render_stage_caption(&self, slot: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let (role, title) = self.block_label(slot);
        let count = self.preset.blocks.len();
        h_flex()
            .flex_shrink_0()
            .h(px(STAGE_CAPTION))
            .gap_2()
            .items_center()
            .justify_center()
            .child(
                Button::new("stage-previous")
                    .small()
                    .ghost()
                    .label("\u{2039}")
                    .tooltip("Previous block (Left)")
                    .disabled(count < 2)
                    .on_click(cx.listener(|this, _, _, cx| this.select_step(-1, cx))),
            )
            .child(
                div()
                    .text_xs()
                    .font_semibold()
                    .text_color(moss())
                    .child(role.to_uppercase()),
            )
            .child(
                div()
                    .font_family(art::PRINT_FONT)
                    .font_semibold()
                    .child(title),
            )
            .child(
                Button::new("stage-next")
                    .small()
                    .ghost()
                    .label("\u{203A}")
                    .tooltip("Next block (Right)")
                    .disabled(count < 2)
                    .on_click(cx.listener(|this, _, _, cx| this.select_step(1, cx))),
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
            - STAGE_CAPTION
            - STAGE_MARGIN;
        let panel = if self.assistant.open {
            assistant_view::PANEL_WIDTH
        } else {
            0.0
        };
        let room_wide = f32::from(viewport.width) - DETAILS_WIDTH - panel - STAGE_MARGIN;
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
            "The chain is empty. Press + Add block to start."
        } else {
            "Click a block in the chain to see its knobs."
        };
        // The picker drops down over the scenes and the stage, so opening it
        // doesn't shrink the unit being worked on.
        let below = v_flex()
            .relative()
            .flex_1()
            .min_h_0()
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
                                Some(slot) => stage.child(
                                    v_flex()
                                        .items_center()
                                        .child(self.render_stage_caption(slot, cx))
                                        .child(self.render_stage_unit(slot, window, cx)),
                                ),
                                None => stage.child(div().text_color(fog()).child(empty)),
                            }),
                    )
                    .when_some(selected, |row, slot| {
                        row.child(self.render_details(slot, cx))
                    }),
            )
            .when_some(self.gear_picker, |below, picker| {
                below.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .shadow_lg()
                        .child(self.render_picker(picker, cx)),
                )
            });
        v_flex()
            .size_full()
            .child(self.render_strip(window, cx))
            .child(below)
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
