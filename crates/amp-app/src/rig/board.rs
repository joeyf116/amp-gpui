//! The pedalboard: gear drawn in signal order, dragged in from the rack or
//! around the board, with live changes on every drop.

use super::{AMP_QUALITY, Rig};
use crate::{
    art::{self, GearArt, Shape, UNIT_HEIGHT},
    style::{self, fog, glow, hairline, moss, moss_glow, pane},
};
use amp_dsp::{AmpInfo, BlockKind};
use amp_preset::{AssetKind, BlockPreset, CaptureEntry, CaptureKind};
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

const GAP: f32 = 22.0;
const FOOTSWITCH_HEIGHT: f32 = 54.0;
const SETTINGS_KNOB: Pixels = px(72.);
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
    Capture(CaptureEntry),
}

impl Gear {
    fn block(&self) -> BlockPreset {
        match self {
            Self::BuiltIn(kind) => BlockPreset::new(*kind, true),
            Self::Capture(capture) => {
                let kind = match capture.kind {
                    CaptureKind::Amp => BlockKind::Amp,
                    CaptureKind::Pedal => BlockKind::Pedal,
                };
                let mut block = BlockPreset::new(kind, true);
                block.asset = Some(capture.asset.clone());
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

fn block_art(block: &BlockPreset, name: Option<&str>) -> GearArt {
    let asset_name = block.asset.as_ref().map(|asset| {
        asset
            .name
            .rsplit_once('.')
            .map_or(asset.name.as_str(), |(stem, _)| stem)
    });
    let title = name
        .or(asset_name)
        .unwrap_or(match block.kind {
            BlockKind::Amp | BlockKind::Pedal => "No capture",
            kind => kind.title(),
        })
        .to_owned();
    let knobs = (0..block.kind.params().len())
        .filter(|index| !block.kind.uses_asset() || *index != AMP_QUALITY)
        .map(|index| {
            let spec = block.kind.params()[index];
            (block.param(index) - spec.min) / (spec.max - spec.min)
        })
        .collect();
    GearArt {
        shape: Shape::for_kind(block.kind, &title),
        title: title.into(),
        knobs,
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
        art::draw(&self.art, true)
            .ml(px(10.))
            .mt(px(10.))
            .opacity(0.88)
            .shadow(vec![glow(0.35, 26.0)])
    }
}

/// The unit a step lands on, wrapping at the ends. With nothing selected, Right
/// starts at the first unit and Left at the last.
fn step_selection(current: Option<usize>, step: isize, count: isize) -> usize {
    let start = current.map_or(if step > 0 { -1 } else { 0 }, |slot| slot as isize);
    (start + step).rem_euclid(count) as usize
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
        match &drag.source {
            DragSource::Rack(gear) => self.insert_block(gear.block(), gap, cx),
            DragSource::Board(from) => self.move_block(*from, landing(*from, gap), cx),
        }
    }

    fn insert_block(&mut self, block: BlockPreset, slot: usize, cx: &mut Context<Self>) {
        self.remember();
        let slot = slot.min(self.preset.blocks.len());
        self.preset.blocks.insert(slot, block);
        self.captures.insert(slot, None);
        self.selected = Some(slot);
        self.rebuild_chain(cx);
    }

    fn move_block(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if from == to || to >= self.preset.blocks.len() {
            return;
        }
        self.remember();
        let block = self.preset.blocks.remove(from);
        self.preset.blocks.insert(to, block);
        let capture = self.captures.remove(from);
        self.captures.insert(to, capture);
        self.selected = Some(to);
        self.rebuild_chain(cx);
    }

    fn remove_block(&mut self, slot: usize, cx: &mut Context<Self>) {
        self.remember();
        self.preset.blocks.remove(slot);
        self.captures.remove(slot);
        self.selected = None;
        self.settings_open = false;
        self.rebuild_chain(cx);
    }

    /// Double-clicking in the rack places gear where it usually goes.
    fn place_block(&mut self, gear: &Gear, cx: &mut Context<Self>) {
        let rank = gear.art().shape.rank();
        let slot = (0..self.preset.blocks.len())
            .find(|slot| self.shape_of(*slot).rank() > rank)
            .unwrap_or(self.preset.blocks.len());
        self.insert_block(gear.block(), slot, cx);
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
        self.preset.blocks = order
            .iter()
            .map(|s| self.preset.blocks[*s].clone())
            .collect();
        self.captures = order.iter().map(|s| self.captures[*s].clone()).collect();
        self.selected = None;
        self.settings_open = false;
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

    pub(super) fn open_selected(&mut self, cx: &mut Context<Self>) {
        self.settings_open = self.selected.is_some();
        cx.notify();
    }

    pub(super) fn close_selected(&mut self, cx: &mut Context<Self>) {
        self.settings_open = false;
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
            if hovered { 118.0_f32 } else { 0.0 },
            cx.theme().motion_tokens().spring_control,
            window,
            cx,
        );
        let cable = 0.22 + (self.output_level * 3.0).min(0.7);
        h_flex()
            .id(("gap", gap))
            .h(px(UNIT_HEIGHT))
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
                    .h(px(UNIT_HEIGHT - 8.))
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
        if self.hover_gap != Some(gap) {
            self.hover_gap = Some(gap);
            cx.notify();
        }
    }

    fn render_unit(&self, slot: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let block = &self.preset.blocks[slot];
        let art = block_art(block, self.block_name(slot));
        let selected = self.selected == Some(slot);
        let enabled = block.enabled;
        let is_stomp = matches!(art.shape, Shape::Stomp(_));
        let drag = BoardDrag {
            source: DragSource::Board(slot),
            art: art.clone(),
        };
        div()
            .id(("unit", slot))
            .relative()
            .rounded_lg()
            .cursor_grab()
            .hover(|unit| unit.shadow(vec![glow(0.18, 14.0)]))
            .when(selected, |unit| unit.shadow(vec![glow(0.4, 18.0)]))
            .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
            .on_drag_move::<BoardDrag>(cx.listener(
                move |this, event: &DragMoveEvent<BoardDrag>, _, cx| {
                    if event.bounds.contains(&event.event.position) {
                        let middle = event.bounds.center().x;
                        let gap = slot + usize::from(event.event.position.x > middle);
                        this.set_hover_gap(gap, cx);
                    }
                },
            ))
            .on_drop(cx.listener(move |this, drag: &BoardDrag, _, cx| {
                let gap = this.hover_gap.unwrap_or(slot);
                this.drop_on_board(drag, gap, cx);
            }))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.selected = Some(slot);
                this.focus.focus(window, cx);
                if event.click_count() >= 2 {
                    this.settings_open = true;
                }
                cx.notify();
            }))
            .child(art::draw(&art, enabled))
            .when(is_stomp, |unit| {
                // The footswitch works like a real one: press it to bypass.
                unit.child(
                    div()
                        .id(("footswitch", slot))
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .h(px(FOOTSWITCH_HEIGHT))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle_block(slot, cx);
                        })),
                )
            })
    }

    fn render_jack(&self, label: &'static str) -> impl IntoElement {
        v_flex()
            .h(px(UNIT_HEIGHT))
            .justify_center()
            .items_center()
            .gap_1()
            .child(
                div()
                    .size(px(12.))
                    .rounded_full()
                    .border_2()
                    .border_color(moss())
                    .shadow(vec![glow(0.4, 10.0)]),
            )
            .child(div().text_xs().text_color(fog()).child(label))
    }

    fn render_board(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        h_flex().flex_wrap().items_center().gap_y_3().children(row)
    }

    fn render_settings(&self, slot: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let block = &self.preset.blocks[slot];
        let kind = block.kind;
        let info = self.captures.get(slot).and_then(Option::as_ref);
        let name = self
            .block_name(slot)
            .map_or_else(|| kind.title().to_owned(), str::to_owned);
        let tiers = info.map_or(1, |info| info.quality_tiers);
        let params = kind.params().len();
        let knobs: Vec<_> = (0..params)
            .filter(|param| !kind.uses_asset() || *param != AMP_QUALITY || tiers > 1)
            .map(|param| self.knob(slot, param, SETTINGS_KNOB, cx))
            .collect();
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
            .id("settings")
            .gap_3()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(hairline())
            .bg(pane())
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(div().text_lg().font_semibold().child(name))
                            .when_some(detail, |column, detail| {
                                column.child(div().text_xs().text_color(fog()).child(detail))
                            }),
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
                            ))
                            .when(
                                AssetKind::for_block(kind) == Some(AssetKind::ImpulseResponse),
                                |row| {
                                    row.child(
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
                                Button::new("remove")
                                    .small()
                                    .ghost()
                                    .label("Remove")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.remove_block(slot, cx);
                                    })),
                            )
                            .child(
                                Button::new("close-settings")
                                    .small()
                                    .ghost()
                                    .label("Close")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.settings_open = false;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .when(doubled_cab, |panel| {
                panel.child(div().text_sm().text_color(cx.theme().warning).child(
                    "One of your captures already includes a cabinet, so you may be hearing two.",
                ))
            })
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_6()
                    .justify_center()
                    .children(knobs),
            )
    }

    pub(super) fn render_board_view(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self
            .selected
            .filter(|slot| self.settings_open && *slot < self.preset.blocks.len());
        v_flex()
            .gap_4()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .text_color(fog())
                            .child("Drag gear to place it. Double-click for settings, press a footswitch to bypass, and Ctrl+Z to undo."),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("undo")
                                    .small()
                                    .ghost()
                                    .label("Undo")
                                    .disabled(self.history.is_empty())
                                    .on_click(cx.listener(|this, _, _, cx| this.undo(cx))),
                            )
                            .child(
                                Button::new("redo")
                                    .small()
                                    .ghost()
                                    .label("Redo")
                                    .disabled(self.future.is_empty())
                                    .on_click(cx.listener(|this, _, _, cx| this.redo(cx))),
                            )
                            .child(
                                Button::new("arrange")
                                    .small()
                                    .ghost()
                                    .label("Arrange in usual order")
                                    .on_click(cx.listener(|this, _, _, cx| this.arrange(cx))),
                            ),
                    ),
            )
            .child(self.render_board(window, cx))
            .when_some(selected, |column, slot| {
                column.child(self.render_settings(slot, cx))
            })
    }

    fn render_rack_item(&self, id: usize, gear: Gear, cx: &mut Context<Self>) -> impl IntoElement {
        let art = gear.art();
        let drag = BoardDrag {
            source: DragSource::Rack(gear.clone()),
            art: art.clone(),
        };
        div()
            .id(("rack", id))
            .px_2()
            .py_1()
            .rounded_md()
            .text_sm()
            .truncate()
            .cursor_grab()
            .hover(|item| item.bg(moss_glow(0.08)))
            .child(art.title.clone())
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
                .map(Gear::Capture)
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
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .gap_px()
            .on_drop(cx.listener(|this, drag: &BoardDrag, _, cx| {
                // Dragging a unit back to the rack takes it off the board.
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
                        .child("Add your own NAM captures here, then drag them onto the board."),
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
