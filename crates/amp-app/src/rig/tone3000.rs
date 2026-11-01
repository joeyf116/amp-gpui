//! TONE3000 in the rig: the partnership splash, sign-in, tone lists, tone
//! details, and loading a model onto the chain. The screens follow the API's
//! design requirements, so check https://www.tone3000.com/api#design-requirements
//! before changing what a screen shows. Nothing is downloaded until someone
//! picks a model, which the terms require.

use super::{Overlay, Rig, board::place_after};
use crate::{
    art,
    settings::data_dir,
    style::{fog, hairline, moss, moss_glow, sheet},
};
use amp_dsp::BlockKind;
use amp_preset::{AssetKind, AssetRef, BlockPreset, CaptureKind, Library, Preset, ToneOrigin};
use amp_tone3000::{Client, Error, Filter, List, Model, Picked, Tone, User, WEBSITE};
use gpui_kit::{
    component::{
        Sizable, StyledExt,
        button::{Button, ButtonVariants},
        h_flex, v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// The publishable key, which is made to ship in apps. The secret key never
/// belongs here or anywhere else in this repo.
const CLIENT_ID: &str = "t3k_pub_OPCInVWS2jZQg5NEfQOXjDidw5VlNK9n";
const SPLASH: &str = "Amp GPUI has partnered with TONE3000 to give you access to a massive \
                      library of Neural Amp Modeler (NAM) captures and IRs of real analog gear, \
                      created by a global community of musicians.";
const TABS: [(List, &str); 5] = [
    (List::Favorites, "Favorites"),
    (List::Created, "Created"),
    (List::Downloaded, "Downloaded"),
    (List::Trending, "Trending"),
    (List::Latest, "Latest"),
];
/// Pixels kept of each picture, which is twice the largest place it shows.
const THUMBNAIL: u32 = 320;
/// Enough to fill a list quickly without leaning on the site.
const IMAGE_WORKERS: usize = 3;
const DESCRIPTION_PREVIEW: usize = 240;
/// The T3K mark's width over its height.
const MARK_ASPECT: f32 = 470.0 / 160.0;
const LOGO_ASPECT: f32 = 1050.0 / 160.0;

#[derive(Default)]
struct Shelf {
    tones: Vec<Tone>,
    page: u32,
    has_next: bool,
    loading: bool,
    loaded: bool,
}

struct Detail {
    tone: Tone,
    models: Vec<Model>,
    expanded: bool,
}

/// What the TONE3000 panel is showing.
pub(super) struct Tone3000 {
    client: Arc<Client>,
    /// Set while the browser is out finishing a sign-in. Storing `true` in it
    /// gives up.
    cancel: Option<Arc<AtomicBool>>,
    user: Option<User>,
    tab: List,
    shelves: HashMap<List, Shelf>,
    detail: Option<Detail>,
    detail_loading: Option<u64>,
    /// The block a picked tone goes into. With none, a block is chosen for it.
    target: Option<usize>,
    /// The model being downloaded.
    busy: Option<u64>,
    images: HashMap<String, Arc<Image>>,
    asked: HashSet<String>,
    queue: VecDeque<String>,
    workers: usize,
    notice: Option<SharedString>,
}

impl Tone3000 {
    pub(super) fn new() -> Self {
        Self {
            client: Arc::new(Client::new(CLIENT_ID, &data_dir())),
            cancel: None,
            user: None,
            tab: List::Favorites,
            shelves: HashMap::new(),
            detail: None,
            detail_loading: None,
            target: None,
            busy: None,
            images: HashMap::new(),
            asked: HashSet::new(),
            queue: VecDeque::new(),
            workers: 0,
            notice: None,
        }
    }
}

/// The block a tone's models run in.
fn block_kind(tone: &Tone) -> BlockKind {
    if tone.is_ir() {
        BlockKind::Cab
    } else if matches!(tone.gear.as_deref(), Some("pedal" | "outboard")) {
        BlockKind::Pedal
    } else {
        BlockKind::Amp
    }
}

/// Whether a tone of `kind` can take over `block`. The built-in amp has no
/// file of its own, so a capture replaces it.
fn fits(block: BlockKind, kind: BlockKind) -> bool {
    block == kind || (block == BlockKind::Tube && kind == BlockKind::Amp)
}

/// Shows the catalog what the block being filled can run.
fn filter_for(block: Option<BlockKind>) -> Filter {
    match block {
        Some(BlockKind::Amp | BlockKind::Tube) => Filter {
            gears: Some("amp_amp-cab"),
            format: Some("nam"),
            architecture: Some("2"),
        },
        Some(BlockKind::Pedal) => Filter {
            gears: Some("pedal"),
            format: Some("nam"),
            architecture: Some("2"),
        },
        Some(BlockKind::Cab) => Filter {
            format: Some("ir"),
            ..Filter::default()
        },
        _ => Filter {
            architecture: Some("2"),
            ..Filter::default()
        },
    }
}

fn origin_of(tone: &Tone, model: &Model) -> ToneOrigin {
    let creator = tone.creator();
    ToneOrigin {
        tone_id: tone.id,
        model_id: model.id,
        title: tone.title.clone(),
        model: model.name.clone(),
        gear: tone.gear_label().to_owned(),
        format: tone.format_label().to_owned(),
        creator: creator.username,
        creator_avatar: creator.avatar_url,
        image: tone.image().map(str::to_owned),
        license: tone.license_label().map(str::to_owned),
    }
}

fn shortened(text: &str, limit: usize) -> Option<String> {
    let end = text.char_indices().nth(limit)?.0;
    Some(format!("{}...", text[..end].trim_end()))
}

/// Downloads a model into the library. The temporary file is parsed there
/// before anything keeps it, so a bad download never reaches the rack.
fn fetch_model(
    client: &Client,
    library: &Library,
    model: &Model,
    kind: BlockKind,
    origin: ToneOrigin,
) -> Result<AssetRef, String> {
    let asset_kind = AssetKind::for_block(kind).unwrap_or(AssetKind::NamModel);
    let file = std::env::temp_dir().join(format!(
        "amp-gpui-{}-{}.{}",
        std::process::id(),
        model.id,
        asset_kind.extension()
    ));
    client
        .download(model, &file)
        .map_err(|error| error.to_string())?;
    let stored = store_model(library, &file, kind, asset_kind, origin);
    let _ = fs::remove_file(&file);
    stored.map_err(|error| error.to_string())
}

fn store_model(
    library: &Library,
    file: &Path,
    kind: BlockKind,
    asset_kind: AssetKind,
    origin: ToneOrigin,
) -> Result<AssetRef, amp_preset::LibraryError> {
    if asset_kind == AssetKind::ImpulseResponse {
        return library.import_asset(asset_kind, file);
    }
    let capture_kind = if kind == BlockKind::Pedal {
        CaptureKind::Pedal
    } else {
        CaptureKind::Amp
    };
    Ok(library.add_tone_capture(file, capture_kind, origin)?.asset)
}

impl Rig {
    /// Runs `work` off the UI thread, then `done` back on it.
    fn with_client<T: Send + 'static>(
        &self,
        cx: &mut Context<Self>,
        work: impl FnOnce(&Client) -> Result<T, Error> + Send + 'static,
        done: impl FnOnce(&mut Self, Result<T, Error>, &mut Context<Self>) + 'static,
    ) {
        let client = Arc::clone(&self.tone3000.client);
        cx.spawn(async move |this, cx| {
            let task = cx.background_executor().spawn(async move { work(&client) });
            let result = task.await;
            let _ = this.update(cx, |this, cx| done(this, result, cx));
        })
        .detach();
    }

    fn tone3000_failed(&mut self, error: Error, cx: &mut Context<Self>) {
        match error {
            Error::Canceled => {}
            Error::SignedOut => {
                self.forget_tone3000();
                self.tone3000.notice = Some(Error::SignedOut.to_string().into());
            }
            error => self.tone3000.notice = Some(error.to_string().into()),
        }
        cx.notify();
    }

    fn forget_tone3000(&mut self) {
        let state = &mut self.tone3000;
        state.user = None;
        state.shelves.clear();
        state.detail = None;
        state.detail_loading = None;
    }

    /// Opens the panel to add a tone, or to change the tone in block `target`.
    pub(super) fn open_tone3000(&mut self, target: Option<usize>, cx: &mut Context<Self>) {
        self.tone3000.target = target;
        self.tone3000.notice = None;
        self.overlay = Overlay::Tone3000;
        if self.tone3000.client.signed_in() {
            self.refresh_tone3000(cx);
        }
        cx.notify();
    }

    fn refresh_tone3000(&mut self, cx: &mut Context<Self>) {
        if self.tone3000.user.is_none() {
            self.with_client(cx, Client::user, |this, result, cx| match result {
                Ok(user) => {
                    if let Some(url) = &user.avatar_url {
                        this.want_image(url.clone(), cx);
                    }
                    this.tone3000.user = Some(user);
                    cx.notify();
                }
                Err(error) => this.tone3000_failed(error, cx),
            });
        }
        self.show_tab(self.tone3000.tab, cx);
    }

    /// Gives up on a sign-in still waiting for the browser.
    pub(super) fn cancel_sign_in(&mut self) {
        if let Some(cancel) = self.tone3000.cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Opens the catalog in the browser, signing in first if needed. It comes
    /// back with a session, and a tone if one was picked.
    fn open_catalog(&mut self, cx: &mut Context<Self>) {
        self.cancel_sign_in();
        let block = self
            .tone3000
            .target
            .and_then(|slot| self.preset.blocks.get(slot))
            .map(|block| block.kind);
        let flow = match self.tone3000.client.start_sign_in(filter_for(block)) {
            Ok(flow) => flow,
            Err(error) => return self.tone3000_failed(error, cx),
        };
        cx.open_url(flow.url());
        let cancel = Arc::new(AtomicBool::new(false));
        self.tone3000.cancel = Some(Arc::clone(&cancel));
        self.tone3000.notice = None;
        let flag = Arc::clone(&cancel);
        self.with_client(
            cx,
            move |client| client.finish_sign_in(&flow, &flag),
            move |this, result, cx| {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                this.tone3000.cancel = None;
                match result {
                    Ok(picked) => this.signed_in(picked, cx),
                    Err(error) => this.tone3000_failed(error, cx),
                }
            },
        );
        cx.notify();
    }

    fn signed_in(&mut self, picked: Picked, cx: &mut Context<Self>) {
        cx.activate(true);
        self.overlay = Overlay::Tone3000;
        self.refresh_tone3000(cx);
        if let Some(tone) = picked.tone_id {
            self.open_tone(tone, self.tone3000.target, cx);
        }
    }

    fn sign_out(&mut self, cx: &mut Context<Self>) {
        self.forget_tone3000();
        if let Err(error) = self.tone3000.client.sign_out() {
            self.tone3000.notice = Some(error.to_string().into());
        }
        cx.notify();
    }

    fn show_tab(&mut self, list: List, cx: &mut Context<Self>) {
        self.tone3000.tab = list;
        self.tone3000.detail = None;
        self.tone3000.detail_loading = None;
        let shelf = self.tone3000.shelves.entry(list).or_default();
        if !shelf.loaded && !shelf.loading {
            self.load_shelf(list, 1, cx);
        }
        cx.notify();
    }

    fn load_shelf(&mut self, list: List, page: u32, cx: &mut Context<Self>) {
        self.tone3000.shelves.entry(list).or_default().loading = true;
        self.with_client(
            cx,
            move |client| client.list(list, page),
            move |this, result, cx| {
                let shelf = this.tone3000.shelves.entry(list).or_default();
                shelf.loading = false;
                match result {
                    Ok(found) => {
                        shelf.loaded = true;
                        shelf.page = found.page.max(page);
                        shelf.has_next = found.has_next();
                        for tone in &found.data {
                            this.want_tone_images(tone, cx);
                        }
                        let shelf = this.tone3000.shelves.entry(list).or_default();
                        shelf.tones.extend(found.data);
                        cx.notify();
                    }
                    Err(error) => this.tone3000_failed(error, cx),
                }
            },
        );
    }

    /// Opens a tone's details. `target` is the block its models would load into.
    pub(super) fn open_tone(&mut self, id: u64, target: Option<usize>, cx: &mut Context<Self>) {
        self.tone3000.target = target;
        self.tone3000.detail = None;
        self.tone3000.detail_loading = Some(id);
        self.overlay = Overlay::Tone3000;
        self.with_client(
            cx,
            move |client| {
                let tone = client.tone(id)?;
                let models = client.models(&tone)?;
                Ok((tone, models))
            },
            move |this, result, cx| {
                if this.tone3000.detail_loading != Some(id) {
                    return;
                }
                this.tone3000.detail_loading = None;
                match result {
                    Ok((tone, models)) => {
                        this.want_tone_images(&tone, cx);
                        this.tone3000.detail = Some(Detail {
                            tone,
                            models,
                            expanded: false,
                        });
                        cx.notify();
                    }
                    Err(error) => this.tone3000_failed(error, cx),
                }
            },
        );
        cx.notify();
    }

    fn want_tone_images(&mut self, tone: &Tone, cx: &mut Context<Self>) {
        if let Some(url) = tone.image() {
            self.want_image(url.to_owned(), cx);
        }
        if let Some(url) = tone.creator().avatar_url {
            self.want_image(url, cx);
        }
    }

    /// Fetches the pictures for tones sitting on the chain.
    pub(super) fn want_block_images(&mut self, cx: &mut Context<Self>) {
        let urls: Vec<String> = self
            .preset
            .blocks
            .iter()
            .filter_map(|block| block.origin.as_ref()?.image.clone())
            .collect();
        for url in urls {
            self.want_image(url, cx);
        }
    }

    fn want_image(&mut self, url: String, cx: &mut Context<Self>) {
        if self.tone3000.asked.insert(url.clone()) {
            self.tone3000.queue.push_back(url);
            self.pump_images(cx);
        }
    }

    fn pump_images(&mut self, cx: &mut Context<Self>) {
        while self.tone3000.workers < IMAGE_WORKERS {
            let Some(url) = self.tone3000.queue.pop_front() else {
                return;
            };
            self.tone3000.workers += 1;
            let wanted = url.clone();
            self.with_client(
                cx,
                move |client| client.thumbnail(&wanted, THUMBNAIL),
                move |this, result, cx| {
                    this.tone3000.workers -= 1;
                    // A missing picture just stays blank.
                    if let Ok(png) = result {
                        let image = Arc::new(Image::from_bytes(ImageFormat::Png, png));
                        this.tone3000.images.insert(url, image);
                        cx.notify();
                    }
                    this.pump_images(cx);
                },
            );
        }
    }

    /// Downloads `model` if it's new here, and puts it on the chain. Picking
    /// another model of a tone already on the chain swaps it in place.
    fn load_model(&mut self, model: Model, cx: &mut Context<Self>) {
        let Some(tone) = self.tone3000.detail.as_ref().map(|detail| &detail.tone) else {
            return;
        };
        let kind = block_kind(tone);
        let origin = origin_of(tone, &model);
        let id = model.id;
        self.tone3000.busy = Some(id);
        self.tone3000.notice = None;
        let client = Arc::clone(&self.tone3000.client);
        let library = self.library.clone();
        let placing = origin.clone();
        cx.spawn(async move |this, cx| {
            let task = cx
                .background_executor()
                .spawn(async move { fetch_model(&client, &library, &model, kind, origin) });
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.tone3000.busy = None;
                match result {
                    Ok(asset) => {
                        this.refresh_catalog();
                        this.place_model(kind, asset, placing, cx);
                    }
                    Err(error) => this.tone3000.notice = Some(error.into()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Picks the block a tone goes in: the one asked for, one already running
    /// this tone, or an empty block of the right type.
    fn slot_for(&self, kind: BlockKind, tone_id: u64) -> Option<usize> {
        let blocks = &self.preset.blocks;
        self.tone3000
            .target
            .filter(|slot| {
                blocks
                    .get(*slot)
                    .is_some_and(|block| fits(block.kind, kind))
            })
            .or_else(|| {
                blocks
                    .iter()
                    .position(|block| block.origin.as_ref().is_some_and(|o| o.tone_id == tone_id))
            })
            .or_else(|| {
                blocks
                    .iter()
                    .position(|block| block.kind == kind && block.asset.is_none())
            })
    }

    fn place_model(
        &mut self,
        kind: BlockKind,
        asset: AssetRef,
        origin: ToneOrigin,
        cx: &mut Context<Self>,
    ) {
        if let Some(url) = &origin.image {
            self.want_image(url.clone(), cx);
        }
        let mut block = BlockPreset::new(kind, true);
        let slot = self.slot_for(kind, origin.tone_id);
        if let Some(current) = slot.map(|slot| &self.preset.blocks[slot])
            && current.kind == kind
        {
            block = current.clone();
        }
        block.asset = Some(asset);
        block.origin = Some(origin);
        match slot {
            Some(slot) => {
                self.remember();
                self.preset.blocks[slot] = block;
                self.selected = Some(slot);
                self.tone3000.target = Some(slot);
                self.rebuild_chain(cx);
            }
            None => {
                let slot = place_after(self, &block);
                self.insert_block(block, slot, cx);
                self.tone3000.target = Some(slot);
            }
        }
    }

    /// A preset can reach this library without its TONE3000 files, because
    /// packs leave them out. They download through the user's own account, then
    /// the preset loads. Returns whether that took over from the caller.
    pub(super) fn restore_tone_models(
        &mut self,
        preset: &Preset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let missing: Vec<(usize, ToneOrigin, BlockKind)> = preset
            .blocks
            .iter()
            .enumerate()
            .filter_map(|(slot, block)| {
                let asset = block.asset.as_ref()?;
                let origin = block.origin.clone()?;
                self.library
                    .asset_path(asset)
                    .is_err()
                    .then_some((slot, origin, block.kind))
            })
            .collect();
        if missing.is_empty() {
            return false;
        }
        if !self.tone3000.client.signed_in() {
            self.overlay = Overlay::Tone3000;
            self.tone3000.notice = Some(
                format!(
                    "\"{}\" uses tones from TONE3000. Sign in, then load it again.",
                    preset.name
                )
                .into(),
            );
            cx.notify();
            return true;
        }
        self.tone3000.notice = Some(format!("Downloading the tones for {}", preset.name).into());
        let client = Arc::clone(&self.tone3000.client);
        let library = self.library.clone();
        let mut preset = preset.clone();
        cx.spawn_in(window, async move |this, cx| {
            let task = cx.background_executor().spawn(async move {
                for (slot, origin, kind) in missing {
                    let model = client
                        .model(origin.model_id)
                        .map_err(|error| error.to_string())?;
                    // The file may have changed since the preset was saved, so
                    // the preset takes whatever arrives.
                    let asset = fetch_model(&client, &library, &model, kind, origin)?;
                    preset.blocks[slot].asset = Some(asset);
                }
                Ok::<_, String>(preset)
            });
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.tone3000.notice = None;
                match result {
                    Ok(preset) => {
                        this.refresh_catalog();
                        this.load_preset(preset, window, cx);
                    }
                    Err(error) => this.set_error(error, cx),
                }
            });
        })
        .detach();
        true
    }

    /// A chain block's picture from its tone, once it has arrived.
    pub(super) fn tone_image(&self, origin: &ToneOrigin) -> Option<Arc<Image>> {
        self.tone3000.images.get(origin.image.as_deref()?).cloned()
    }

    /// The tone's picture, or a blank square until it arrives.
    fn picture(&self, url: Option<&str>, size: f32) -> Div {
        div()
            .size(px(size))
            .flex_shrink_0()
            .rounded_md()
            .overflow_hidden()
            .bg(moss_glow(0.08))
            .when_some(
                url.and_then(|url| self.tone3000.images.get(url)).cloned(),
                |frame, image| frame.child(img(image).size_full().object_fit(ObjectFit::Cover)),
            )
    }

    fn avatar(&self, url: Option<&str>, size: f32) -> Div {
        self.picture(url, size).rounded_full()
    }

    /// Shows on a chain block that runs a TONE3000 tone.
    pub(super) fn tone_badge(&self, origin: &ToneOrigin) -> Div {
        v_flex()
            .absolute()
            .top(px(2.))
            .left(px(2.))
            .gap(px(2.))
            .items_start()
            .child(self.picture(origin.image.as_deref(), 28.0))
            .child(mark(8.0))
    }

    /// Credits the tone in the block's details, with what the panel has room for.
    pub(super) fn render_tone_credit(
        &self,
        origin: &ToneOrigin,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(self.picture(origin.image.as_deref(), 44.0))
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(div().text_xs().truncate().child(origin.model.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(fog())
                                    .child(format!("{}  /  {}", origin.gear, origin.format)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(fog())
                                    .truncate()
                                    .child(format!("by {}", origin.creator)),
                            ),
                    ),
            )
            .child(mark(10.0))
            .child(
                Button::new("tone-details")
                    .small()
                    .outline()
                    .label("Tone details")
                    .on_click({
                        let id = origin.tone_id;
                        let slot = self.selected;
                        cx.listener(move |this, _, _, cx| this.open_tone(id, slot, cx))
                    }),
            )
    }

    fn render_splash(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_6()
            .p_6()
            .child(
                h_flex()
                    .gap_4()
                    .items_center()
                    .child(img(art::logo()).size(px(52.)))
                    .child(logo(38.0)),
            )
            .child(
                div()
                    .max_w(px(520.))
                    .text_center()
                    .text_color(fog())
                    .child(SPLASH),
            )
            .child(
                Button::new("t3k-continue")
                    .primary()
                    .label("Continue")
                    .on_click(cx.listener(|this, _, _, cx| this.open_catalog(cx))),
            )
    }

    fn render_waiting(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_4()
            .child(logo(30.0))
            .child(
                div()
                    .text_color(fog())
                    .child("Finish in your browser. This window picks up when you're done."),
            )
            .child(
                Button::new("t3k-cancel")
                    .small()
                    .ghost()
                    .label("Cancel")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.cancel_sign_in();
                        cx.notify();
                    })),
            )
    }

    fn render_tone_card(&self, index: usize, tone: &Tone, cx: &mut Context<Self>) -> AnyElement {
        let creator = tone.creator();
        let id = tone.id;
        h_flex()
            .id(("t3k-tone", index))
            .gap_3()
            .items_center()
            .p_2()
            .rounded_md()
            .cursor_pointer()
            .hover(|row| row.bg(moss_glow(0.06)))
            .child(self.picture(tone.image(), 64.0))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(div().font_semibold().truncate().child(tone.title.clone()))
                    .child(div().text_xs().text_color(fog()).child(format!(
                        "{}  /  {}",
                        tone.gear_label(),
                        tone.format_label()
                    )))
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(self.avatar(creator.avatar_url.as_deref(), 16.0))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(fog())
                                    .truncate()
                                    .child(creator.username),
                            ),
                    ),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_tone(id, this.tone3000.target, cx);
            }))
            .into_any_element()
    }

    fn render_shelf_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = self.tone3000.tab;
        let shelf = self.tone3000.shelves.get(&tab);
        let tones = shelf.map_or(&[][..], |shelf| shelf.tones.as_slice());
        let loading = shelf.is_some_and(|shelf| shelf.loading);
        let has_next = shelf.is_some_and(|shelf| shelf.has_next);
        let next_page = shelf.map_or(1, |shelf| shelf.page + 1);
        let cards: Vec<AnyElement> = tones
            .iter()
            .enumerate()
            .map(|(index, tone)| self.render_tone_card(index, tone, cx))
            .collect();
        let empty = cards.is_empty() && !loading;
        v_flex()
            .id("t3k-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_2()
            .gap_px()
            .when(empty, |list| {
                list.child(
                    div()
                        .p_3()
                        .text_sm()
                        .text_color(fog())
                        .child("Nothing here yet. Browse TONE3000 to find a tone."),
                )
            })
            .children(cards)
            .when(loading, |list| {
                list.child(div().p_3().text_sm().text_color(fog()).child("Loading"))
            })
            .when(has_next && !loading, |list| {
                list.child(
                    Button::new("t3k-more")
                        .small()
                        .ghost()
                        .label("Show more")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.load_shelf(tab, next_page, cx);
                            cx.notify();
                        })),
                )
            })
            // The design requirements ask partial lists to lead to the catalog.
            .when(tab.is_feed() && !tones.is_empty(), |list| {
                list.child(
                    h_flex()
                        .id("t3k-discover")
                        .gap_3()
                        .items_center()
                        .p_3()
                        .mt_1()
                        .rounded_md()
                        .border_1()
                        .border_color(hairline())
                        .cursor_pointer()
                        .hover(|row| row.bg(moss_glow(0.06)))
                        .child(mark(12.0))
                        .child(div().text_sm().child("Discover more tones on TONE3000"))
                        .on_click(cx.listener(|this, _, _, cx| this.open_catalog(cx))),
                )
            })
    }

    fn render_model_row(
        &self,
        index: usize,
        model: &Model,
        tone: &Tone,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let loaded = self.preset.blocks.iter().any(|block| {
            block
                .origin
                .as_ref()
                .is_some_and(|origin| origin.model_id == model.id)
        });
        let busy = self.tone3000.busy == Some(model.id);
        let free = self.tone3000.busy.is_none() && tone.can_load();
        let status = if busy {
            "Downloading"
        } else if loaded {
            "On the chain"
        } else {
            ""
        };
        let picked = model.clone();
        h_flex()
            .id(("t3k-model", index))
            .gap_3()
            .items_center()
            .justify_between()
            .px_2()
            .py_1()
            .rounded_md()
            .when(loaded, |row| row.bg(moss_glow(0.12)))
            .when(free, |row| {
                row.cursor_pointer()
                    .hover(|row| row.bg(moss_glow(0.06)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.load_model(picked.clone(), cx);
                    }))
            })
            .child(
                v_flex()
                    .min_w_0()
                    .child(
                        div()
                            .text_sm()
                            .truncate()
                            .when(loaded, |name| name.text_color(moss()))
                            .child(model.name.clone()),
                    )
                    .child(div().text_xs().text_color(fog()).child(model.detail())),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(fog())
                    .child(status),
            )
            .into_any_element()
    }

    fn render_detail(&self, detail: &Detail, cx: &mut Context<Self>) -> impl IntoElement {
        let tone = &detail.tone;
        let creator = tone.creator();
        let description = tone.description.clone().unwrap_or_default();
        let preview = shortened(&description, DESCRIPTION_PREVIEW);
        let shown = match (&preview, detail.expanded) {
            (Some(preview), false) => preview.clone(),
            _ => description.clone(),
        };
        let can_expand = preview.is_some() && !detail.expanded;
        let models: Vec<AnyElement> = detail
            .models
            .iter()
            .enumerate()
            .map(|(index, model)| self.render_model_row(index, model, tone, cx))
            .collect();
        let site = tone.url.clone();
        h_flex()
            .flex_1()
            .min_h_0()
            .items_start()
            .child(
                v_flex()
                    .id("t3k-about")
                    .w(px(340.))
                    .h_full()
                    .flex_shrink_0()
                    .overflow_y_scroll()
                    .gap_3()
                    .p_3()
                    .border_r_1()
                    .border_color(hairline())
                    .child(
                        Button::new("t3k-back")
                            .small()
                            .ghost()
                            .label("Back to the list")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_tab(this.tone3000.tab, cx);
                            })),
                    )
                    .child(self.picture(tone.image(), 316.0))
                    .child(div().text_lg().font_semibold().child(tone.title.clone()))
                    .child(div().text_sm().text_color(fog()).child(format!(
                        "{}  /  {}",
                        tone.gear_label(),
                        tone.format_label()
                    )))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(self.avatar(creator.avatar_url.as_deref(), 24.0))
                            .child(div().text_sm().child(creator.username)),
                    )
                    .when_some(tone.license_label(), |column, license| {
                        column.child(
                            div()
                                .text_xs()
                                .text_color(fog())
                                .child(format!("Shared under {license}")),
                        )
                    })
                    .when(!shown.is_empty(), |column| {
                        column.child(div().text_sm().text_color(fog()).child(shown))
                    })
                    .when(can_expand, |column| {
                        column.child(
                            Button::new("t3k-read-more")
                                .small()
                                .ghost()
                                .label("Read more")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(detail) = &mut this.tone3000.detail {
                                        detail.expanded = true;
                                    }
                                    cx.notify();
                                })),
                        )
                    })
                    .when(!site.is_empty(), |column| {
                        column.child(
                            Button::new("t3k-site")
                                .small()
                                .ghost()
                                .label("View on TONE3000")
                                .on_click(move |_, _, cx| cx.open_url(&site)),
                        )
                    }),
            )
            .child(
                v_flex()
                    .id("t3k-models")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .gap_px()
                    .p_3()
                    .child(div().mb_2().font_semibold().child("Models in this pack"))
                    .when(!tone.can_load(), |list| {
                        list.child(div().mb_2().text_sm().text_color(fog()).child(format!(
                            "Amp GPUI can't run {} files, so these can't load.",
                            tone.format_label()
                        )))
                    })
                    .when(models.is_empty(), |list| {
                        list.child(
                            div()
                                .text_sm()
                                .text_color(fog())
                                .child("This tone has no models to load yet."),
                        )
                    })
                    .children(models),
            )
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs: Vec<AnyElement> = TABS
            .iter()
            .enumerate()
            .map(|(index, (list, title))| {
                let list = *list;
                let current = self.tone3000.tab == list && self.tone3000.detail.is_none();
                div()
                    .id(("t3k-tab", index))
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .text_sm()
                    .cursor_pointer()
                    .when(current, |tab| tab.bg(moss_glow(0.14)).text_color(moss()))
                    .when(!current, |tab| tab.hover(|tab| tab.bg(moss_glow(0.06))))
                    .child(*title)
                    .on_click(cx.listener(move |this, _, _, cx| this.show_tab(list, cx)))
                    .into_any_element()
            })
            .collect();
        h_flex()
            .gap_1()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(hairline())
            .children(tabs)
    }

    fn render_browse(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let user = self.tone3000.user.as_ref();
        let name = user.map_or_else(String::new, |user| user.username.clone());
        let avatar = user.and_then(|user| user.avatar_url.as_deref());
        let body = match (&self.tone3000.detail, self.tone3000.detail_loading) {
            (Some(detail), _) => self.render_detail(detail, cx).into_any_element(),
            (None, Some(_)) => div()
                .flex_1()
                .p_4()
                .text_color(fog())
                .child("Loading")
                .into_any_element(),
            (None, None) => self.render_shelf_list(cx).into_any_element(),
        };
        v_flex()
            .flex_1()
            .min_h_0()
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .p_3()
                    .border_b_1()
                    .border_color(hairline())
                    .child(logo(20.0))
                    .child(div().flex_1())
                    .child(
                        Button::new("t3k-browse")
                            .small()
                            .outline()
                            .child(mark(10.0))
                            .label("Browse TONE3000")
                            .on_click(cx.listener(|this, _, _, cx| this.open_catalog(cx))),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(self.avatar(avatar, 24.0))
                            .child(div().text_sm().child(name)),
                    )
                    .child(
                        Button::new("t3k-sign-out")
                            .small()
                            .ghost()
                            .label("Sign out")
                            .on_click(cx.listener(|this, _, _, cx| this.sign_out(cx))),
                    )
                    .child(close_button(cx)),
            )
            .child(self.render_tabs(cx))
            .child(body)
    }

    pub(super) fn render_tone3000(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let state = &self.tone3000;
        let waiting = state.cancel.is_some();
        let signed_in = state.client.signed_in();
        v_flex()
            .id("tone3000")
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
            .map(|panel| {
                if waiting {
                    panel.child(self.render_waiting(cx))
                } else if signed_in {
                    panel.child(self.render_browse(cx))
                } else {
                    panel
                        .child(h_flex().justify_end().p_2().child(close_button(cx)))
                        .child(div().flex_1().min_h_0().child(self.render_splash(cx)))
                }
            })
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(hairline())
                    .child(
                        div()
                            .text_sm()
                            .text_color(fog())
                            .truncate()
                            .children(state.notice.clone()),
                    )
                    .child(
                        div()
                            .id("t3k-powered-by")
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(fog())
                            .cursor_pointer()
                            .hover(|link| link.text_color(moss()))
                            .child("Powered by TONE3000")
                            .on_click(|_, _, cx| cx.open_url(WEBSITE)),
                    ),
            )
    }
}

fn close_button(cx: &mut Context<Rig>) -> impl IntoElement + use<> {
    Button::new("close-tone3000")
        .small()
        .ghost()
        .label("Close")
        .on_click(cx.listener(|this, _, window, cx| this.close_overlay(window, cx)))
}

fn logo(height: f32) -> impl IntoElement {
    img(art::tone3000_logo())
        .h(px(height))
        .w(px(height * LOGO_ASPECT))
        .flex_shrink_0()
}

fn mark(height: f32) -> Img {
    img(art::t3k_mark())
        .h(px(height))
        .w(px(height * MARK_ASPECT))
        .flex_shrink_0()
}

#[cfg(test)]
mod tests {
    use super::{BlockKind, Model, Tone, block_kind, filter_for, fits, origin_of, shortened};

    fn tone(json: &str) -> Tone {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn tones_go_in_the_block_that_can_run_them() {
        assert_eq!(
            block_kind(&tone(r#"{"id":1,"format":"ir"}"#)),
            BlockKind::Cab
        );
        assert_eq!(
            block_kind(&tone(r#"{"id":1,"format":"nam","gear":"pedal"}"#)),
            BlockKind::Pedal
        );
        assert_eq!(
            block_kind(&tone(r#"{"id":1,"format":"nam","gear":"amp-cab"}"#)),
            BlockKind::Amp
        );
    }

    #[test]
    fn a_capture_can_replace_the_built_in_amp_but_not_a_pedal() {
        assert!(fits(BlockKind::Tube, BlockKind::Amp));
        assert!(fits(BlockKind::Cab, BlockKind::Cab));
        assert!(!fits(BlockKind::Drive, BlockKind::Pedal));
        assert!(!fits(BlockKind::Cab, BlockKind::Amp));
    }

    #[test]
    fn the_catalog_shows_what_the_block_can_run() {
        let amp = filter_for(Some(BlockKind::Tube));
        assert_eq!(amp.gears, Some("amp_amp-cab"));
        assert_eq!(amp.architecture, Some("2"));
        assert_eq!(filter_for(Some(BlockKind::Pedal)).gears, Some("pedal"));
        let cab = filter_for(Some(BlockKind::Cab));
        assert_eq!(cab.format, Some("ir"));
        assert_eq!(cab.architecture, None);
        assert_eq!(filter_for(None).architecture, Some("2"));
    }

    #[test]
    fn an_origin_keeps_the_credit_and_license() {
        let tone = tone(
            r#"{"id":7,"title":"Plexi","gear":"amp","format":"nam","license":"cc-by",
                "images":["https://x/a.png"],"user":{"username":"brucew"}}"#,
        );
        let model: Model = serde_json::from_str(r#"{"id":9,"name":"DI 3"}"#).unwrap();
        let origin = origin_of(&tone, &model);
        assert_eq!(origin.tone_id, 7);
        assert_eq!(origin.model_id, 9);
        assert_eq!(origin.creator, "brucew");
        assert_eq!(origin.license.as_deref(), Some("CC BY"));
        assert_eq!(origin.image.as_deref(), Some("https://x/a.png"));
    }

    #[test]
    fn long_descriptions_shorten_on_a_character_boundary() {
        assert_eq!(shortened("short", 10), None);
        assert_eq!(shortened("héllo wörld", 5).unwrap(), "héllo...");
    }
}
