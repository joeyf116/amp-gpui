//! The tone assistant in the rig: chats, trying suggestions, and learning from
//! what the player does with them. Claude runs on its own thread and reports
//! through a channel that the meter tick drains, so the UI never waits on it, and
//! Stop can take effect at once whatever the network is doing.
//!
//! Nothing Claude says changes the rig. A press does, and that press is what
//! allows a TONE3000 download under their terms.

use super::{Overlay, Rig, tone3000::fetch_for_block};
use crate::settings::data_dir;
use amp_assistant::{
    Chat, ChatStore, ChatSummary, Client, Error, Event, Host, ModelChoice, PickerRequest, Proposal,
    Request, Role, Session, Snapshot, Taste, ToneCatalog, Turn, Usage, folder, key_file,
    needs_download, run_turn,
};
use amp_dsp::BlockKind;
use amp_engine::ENGINE_RATE;
use amp_preset::{BlockPreset, Library, Preset, loudness, trim_to};
use amp_secret::SealedFile;
use gpui_kit::{
    component::input::{InputEvent, InputState, TextareaState},
    *,
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    thread,
};

/// Which page of the panel is showing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Page {
    Chat,
    History,
    Taste,
    Settings,
}

/// A reply being written.
pub(super) struct Running {
    cancel: Arc<AtomicBool>,
    events: Receiver<Event>,
}

/// A suggestion on the rig, not yet kept. It holds both sides of the comparison,
/// so A/B needs nothing from the library.
pub(super) struct Trial {
    /// The turn and suggestion it came from.
    pub(super) at: (usize, usize),
    before: Preset,
    after: Preset,
    /// The blocks as suggested, to see how the player adjusts them.
    suggested: Vec<BlockPreset>,
    tags: Vec<String>,
    pub(super) hearing_after: bool,
    pub(super) trimmed: Option<f32>,
    edits_current: bool,
}

/// What the assistant panel is showing.
pub(super) struct Assistant {
    pub(super) open: bool,
    pub(super) page: Page,
    pub(super) key: Option<String>,
    key_file: SealedFile,
    pub(super) key_input: Entity<InputState>,
    pub(super) checking_key: bool,
    pub(super) input: Entity<TextareaState>,
    pub(super) chat: Chat,
    pub(super) session: Session,
    store: ChatStore,
    pub(super) chats: Vec<ChatSummary>,
    pub(super) taste: Taste,
    folder: PathBuf,
    pub(super) running: Option<Running>,
    pub(super) status: Option<&'static str>,
    pub(super) failure: Option<Error>,
    pub(super) notice: Option<SharedString>,
    pub(super) picker: Option<PickerRequest>,
    /// Set while the TONE3000 picker is open on Claude's behalf.
    awaiting_pick: bool,
    pub(super) usage: Usage,
    pub(super) trial: Option<Trial>,
    /// The suggestion getting ready to try.
    pub(super) preparing: Option<(usize, usize)>,
    pub(super) scroll: ScrollHandle,
    pub(super) renaming: Option<Entity<InputState>>,
}

impl Assistant {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Rig>) -> (Self, Subscription) {
        let data = data_dir();
        let home = folder(&data);
        let key_file = key_file(&data);
        let key = key_file
            .load()
            .and_then(|bytes| String::from_utf8(bytes).ok());
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 6)
                .submit_on_enter(true)
                .placeholder("Ask for a sound, or how to change this one")
        });
        let key_input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("sk-ant-...")
        });
        let subscription =
            cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { shift: false, .. } = event {
                    this.send_from_input(window, cx);
                }
            });
        let store = ChatStore::new(&home);
        let assistant = Self {
            open: false,
            page: Page::Chat,
            key,
            key_file,
            key_input,
            checking_key: false,
            input,
            chat: Chat::new(),
            session: Session::default(),
            chats: store.list(),
            store,
            taste: Taste::load(&home),
            folder: home,
            running: None,
            status: None,
            failure: None,
            notice: None,
            picker: None,
            awaiting_pick: false,
            usage: Usage::default(),
            trial: None,
            preparing: None,
            scroll: ScrollHandle::new(),
            renaming: None,
        };
        (assistant, subscription)
    }
}

/// How a turn gets back to the rig.
struct Worker {
    events: Sender<Event>,
    catalog: Arc<amp_tone3000::Client>,
    cancel: Arc<AtomicBool>,
}

impl Host for Worker {
    fn event(&self, event: Event) {
        // The panel may have stopped listening, which is fine.
        let _ = self.events.send(event);
    }

    fn catalog(&self) -> &dyn ToneCatalog {
        self.catalog.as_ref()
    }

    fn canceled(&self) -> &AtomicBool {
        &self.cancel
    }
}

/// `name`, or the first free name after it. A suggestion never replaces a tone
/// the player already has.
fn unique_name(name: &str, taken: &[&str]) -> String {
    let free = |candidate: &str| {
        !taken
            .iter()
            .any(|other| other.eq_ignore_ascii_case(candidate))
    };
    if free(name) {
        return name.to_owned();
    }
    (2..)
        .map(|number| format!("{name} {number}"))
        .find(|candidate| free(candidate))
        .unwrap_or_else(|| name.to_owned())
}

/// Downloads what a suggestion needs, and turns it down if it would jump out
/// over `reference`, which is what's playing now. Runs off the UI thread.
fn prepare_trial(
    mut preset: Preset,
    reference: &Preset,
    client: &amp_tone3000::Client,
    library: &Library,
) -> Result<(Preset, Option<f32>), String> {
    for block in preset
        .blocks
        .iter_mut()
        .filter(|block| needs_download(block))
    {
        if let Some(origin) = block.origin.clone() {
            block.asset = Some(fetch_for_block(client, library, origin, block.kind)?);
        }
    }
    let measured = loudness(&preset, library, ENGINE_RATE).map_err(|error| error.to_string())?;
    let trimmed = loudness(reference, library, ENGINE_RATE)
        .ok()
        .and_then(|level| trim_to(&mut preset, measured, level));
    Ok((preset, trimmed))
}

impl Rig {
    pub(super) fn toggle_assistant(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.assistant.open = !self.assistant.open;
        if self.assistant.open {
            self.assistant.page = Page::Chat;
            let input = self.assistant.input.read(cx).focus_handle(cx);
            input.focus(window, cx);
        } else {
            self.focus.focus(window, cx);
        }
        cx.notify();
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            preset: self.preset.clone(),
            selected: self.selected,
            tones: self
                .presets
                .iter()
                .map(|entry| entry.preset.clone())
                .collect(),
            captures: self.catalog.clone(),
            tone3000_signed_in: self.tone3000_client().signed_in(),
        }
    }

    fn save_chat(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = self.assistant.store.save(&self.assistant.chat) {
            return self.set_error(error, cx);
        }
        self.assistant.chats = self.assistant.store.list();
    }

    fn save_taste(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = self.assistant.taste.save(&self.assistant.folder) {
            self.set_error(error, cx);
        }
    }

    pub(super) fn send_from_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.assistant.input.read(cx).value().trim().to_owned();
        if text.is_empty() || self.assistant.running.is_some() {
            return;
        }
        self.assistant
            .input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.send(text, cx);
    }

    /// Starts a turn on its own thread. With no key yet, the settings come first.
    pub(super) fn send(&mut self, text: String, cx: &mut Context<Self>) {
        let Some(key) = self.assistant.key.clone() else {
            self.assistant.page = Page::Settings;
            return cx.notify();
        };
        let snapshot = self.snapshot();
        let state = &mut self.assistant;
        state.failure = None;
        state.picker = None;
        state.page = Page::Chat;
        state.chat.name_from(&text);
        state.chat.turns.push(Turn::user(text.as_str()));
        state.chat.turns.push(Turn::assistant());
        state.session.add_user(&text, &snapshot);
        self.start_turn(key, snapshot, cx);
    }

    fn start_turn(&mut self, key: String, snapshot: Snapshot, cx: &mut Context<Self>) {
        let (sender, events) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let request = Request {
            model: self.settings.assistant_model,
            taste: self.assistant.taste.clone(),
            messages: self.assistant.session.messages.clone(),
            snapshot,
        };
        let worker = Worker {
            events: sender,
            catalog: self.tone3000_client(),
            cancel: Arc::clone(&cancel),
        };
        self.assistant.running = Some(Running { cancel, events });
        self.assistant.status = Some("Thinking");
        self.assistant.scroll.scroll_to_bottom();
        thread::spawn(move || run_turn(request, &Client::new(&key), &worker));
        cx.notify();
    }

    /// Stops listening right away. The partial reply stays, and whatever else
    /// the thread sends goes nowhere.
    pub(super) fn stop_reply(&mut self, cx: &mut Context<Self>) {
        if let Some(running) = self.assistant.running.take() {
            running.cancel.store(true, Ordering::Relaxed);
            self.assistant.status = None;
            self.finish_partial();
            self.save_chat(cx);
            cx.notify();
        }
    }

    fn finish_partial(&mut self) {
        let text = self
            .assistant
            .chat
            .turns
            .last()
            .filter(|turn| turn.role == Role::Assistant)
            .map_or_else(String::new, |turn| turn.text.clone());
        self.assistant.session.keep_partial(&text);
    }

    /// Takes whatever Claude has sent since the last tick.
    pub(super) fn pump_assistant(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        while let Some(running) = &self.assistant.running {
            match running.events.try_recv() {
                Ok(event) => {
                    self.take_event(event, cx);
                    changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.assistant.running = None;
                    self.assistant.status = None;
                    break;
                }
            }
        }
        if changed {
            self.follow_reply();
            cx.notify();
        }
    }

    /// Stays at the end of the chat, unless the player has scrolled up to read.
    fn follow_reply(&self) {
        let scroll = &self.assistant.scroll;
        if scroll.offset().y <= -scroll.max_offset().y + px(80.) {
            scroll.scroll_to_bottom();
        }
    }

    fn reply_turn(&mut self) -> Option<&mut Turn> {
        self.assistant
            .chat
            .turns
            .last_mut()
            .filter(|turn| turn.role == Role::Assistant)
    }

    fn take_event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Text(text) => {
                self.assistant.status = None;
                if let Some(turn) = self.reply_turn() {
                    turn.text.push_str(&text);
                }
            }
            Event::Status(status) => self.assistant.status = Some(status),
            Event::Suggested(proposal) => {
                let images: Vec<String> = proposal
                    .credits()
                    .filter_map(|credit| credit.image.clone())
                    .collect();
                for image in images {
                    self.want_image(image, cx);
                }
                if let Some(turn) = self.reply_turn() {
                    turn.proposals.push(*proposal);
                }
            }
            Event::Picker(picker) => self.assistant.picker = Some(picker),
            Event::Remembered(fact) => {
                let learning = self.assistant.taste.learning;
                if learning && self.assistant.taste.remember(&fact) {
                    self.save_taste(cx);
                    if let Some(turn) = self.reply_turn() {
                        turn.remembered.push(fact);
                    }
                }
            }
            Event::Usage(usage) => self.assistant.usage.add(usage),
            Event::Done(messages) => {
                self.assistant.session.take(messages);
                self.turn_ended(cx);
            }
            Event::Failed(error) => {
                self.finish_partial();
                if error != Error::Canceled {
                    if error == Error::BadKey {
                        self.assistant.page = Page::Settings;
                    }
                    self.assistant.failure = Some(error);
                }
                self.turn_ended(cx);
            }
        }
    }

    fn turn_ended(&mut self, cx: &mut Context<Self>) {
        self.assistant.running = None;
        self.assistant.status = None;
        self.save_chat(cx);
    }

    /// The player's last message, with everything after it taken back.
    fn take_back_last(&mut self) -> Option<String> {
        let state = &mut self.assistant;
        let at = state
            .chat
            .turns
            .iter()
            .rposition(|turn| turn.role == Role::User)?;
        let text = state.chat.turns[at].text.clone();
        state.chat.turns.truncate(at);
        state.session.rewind();
        state.failure = None;
        Some(text)
    }

    pub(super) fn regenerate(&mut self, cx: &mut Context<Self>) {
        self.stop_reply(cx);
        if let Some(text) = self.take_back_last() {
            self.send(text, cx);
        }
    }

    /// Puts the last message back in the box to change and send again.
    pub(super) fn edit_last(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.stop_reply(cx);
        if let Some(text) = self.take_back_last() {
            self.assistant
                .input
                .update(cx, |input, cx| input.set_value(text, window, cx));
            let input = self.assistant.input.read(cx).focus_handle(cx);
            input.focus(window, cx);
            self.save_chat(cx);
            cx.notify();
        }
    }

    pub(super) fn new_chat(&mut self, cx: &mut Context<Self>) {
        self.stop_reply(cx);
        let state = &mut self.assistant;
        state.chat = Chat::new();
        state.session = Session::default();
        state.failure = None;
        state.picker = None;
        state.page = Page::Chat;
        cx.notify();
    }

    pub(super) fn open_chat(&mut self, id: u64, cx: &mut Context<Self>) {
        self.stop_reply(cx);
        let state = &mut self.assistant;
        if let Some(chat) = state.store.load(id) {
            state.session = Session::resume(chat.replay());
            state.chat = chat;
            state.failure = None;
            state.picker = None;
            state.page = Page::Chat;
        }
        cx.notify();
    }

    pub(super) fn delete_chat(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.assistant.chat.id == id {
            self.new_chat(cx);
            self.assistant.page = Page::History;
        }
        if let Err(error) = self.assistant.store.delete(id) {
            self.set_error(error, cx);
        }
        self.assistant.chats = self.assistant.store.list();
        cx.notify();
    }

    pub(super) fn start_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = self.assistant.chat.title.clone();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(title));
        cx.subscribe_in(&input, window, |this, _, event: &InputEvent, _, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.finish_rename(cx);
            }
        })
        .detach();
        let focus = input.read(cx).focus_handle(cx);
        focus.focus(window, cx);
        self.assistant.renaming = Some(input);
        cx.notify();
    }

    fn finish_rename(&mut self, cx: &mut Context<Self>) {
        if let Some(input) = self.assistant.renaming.take() {
            let title = input.read(cx).value().trim().to_owned();
            if !title.is_empty() {
                self.assistant.chat.title = title;
                self.save_chat(cx);
            }
            cx.notify();
        }
    }

    pub(super) fn set_model(&mut self, model: ModelChoice, cx: &mut Context<Self>) {
        self.settings.assistant_model = model;
        self.save_settings(cx);
        cx.notify();
    }

    /// Checks a pasted key before keeping it. A network failure keeps the key
    /// anyway, since it says nothing about the key.
    pub(super) fn save_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.assistant.key_input.read(cx).value().trim().to_owned();
        if key.is_empty() {
            return;
        }
        self.assistant.checking_key = true;
        self.assistant.notice = None;
        self.assistant
            .key_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        let checking = key.clone();
        cx.spawn(async move |this, cx| {
            let task = cx
                .background_executor()
                .spawn(async move { Client::new(&checking).check_key() });
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.assistant.checking_key = false;
                match result {
                    Err(error @ (Error::BadKey | Error::NoCredit)) => {
                        this.assistant.notice = Some(error.to_string().into());
                    }
                    other => this.keep_key(key, other.err(), cx),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn keep_key(&mut self, key: String, problem: Option<Error>, cx: &mut Context<Self>) {
        // Without a safe place for it, the key lasts until the app closes.
        if let Err(error) = self.assistant.key_file.save(key.as_bytes())
            && error.kind() != std::io::ErrorKind::Unsupported
        {
            self.set_error(error, cx);
        }
        self.assistant.key = Some(key);
        self.assistant.notice = problem.map(|error| error.to_string().into());
        if self.assistant.notice.is_none() {
            self.assistant.page = Page::Chat;
        }
    }

    pub(super) fn remove_key(&mut self, cx: &mut Context<Self>) {
        self.stop_reply(cx);
        self.assistant.key = None;
        if let Err(error) = self.assistant.key_file.clear() {
            self.set_error(error, cx);
        }
        cx.notify();
    }

    pub(super) fn forget_fact(&mut self, index: usize, cx: &mut Context<Self>) {
        self.assistant.taste.forget(index);
        self.save_taste(cx);
        cx.notify();
    }

    pub(super) fn clear_taste(&mut self, cx: &mut Context<Self>) {
        self.assistant.taste.clear();
        self.save_taste(cx);
        cx.notify();
    }

    pub(super) fn toggle_taste_learning(&mut self, cx: &mut Context<Self>) {
        self.assistant.taste.learning = !self.assistant.taste.learning;
        self.save_taste(cx);
        cx.notify();
    }

    fn proposal_at(&self, at: (usize, usize)) -> Option<&Proposal> {
        self.assistant.chat.turns.get(at.0)?.proposals.get(at.1)
    }

    /// Gets a suggestion ready and puts it on the rig. Pressing Try is what
    /// asks for any download.
    pub(super) fn try_proposal(
        &mut self,
        at: (usize, usize),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(proposal) = self.proposal_at(at).cloned() else {
            return;
        };
        let client = self.tone3000_client();
        if proposal.downloads() > 0 && !client.signed_in() {
            self.assistant.notice = Some("Sign in to TONE3000 to get this model.".into());
            return self.open_tone3000(None, cx);
        }
        self.assistant.preparing = Some(at);
        self.assistant.notice = None;
        let reference = self
            .assistant
            .trial
            .as_ref()
            .map_or_else(|| self.preset.clone(), |trial| trial.before.clone());
        let library = self.library.clone();
        let preset = proposal.preset.clone();
        let measured_against = reference.clone();
        cx.spawn_in(window, async move |this, cx| {
            let task = cx
                .background_executor()
                .spawn(async move { prepare_trial(preset, &measured_against, &client, &library) });
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.assistant.preparing = None;
                this.refresh_catalog();
                match result {
                    Ok((preset, trimmed)) => {
                        this.start_trial(at, &proposal, reference, preset, trimmed, window, cx);
                    }
                    Err(error) => this.assistant.notice = Some(error.into()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    #[expect(clippy::too_many_arguments, reason = "one call site, all distinct")]
    fn start_trial(
        &mut self,
        at: (usize, usize),
        proposal: &Proposal,
        before: Preset,
        preset: Preset,
        trimmed: Option<f32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut after = preset;
        if proposal.edits_current {
            after.name.clone_from(&before.name);
            after.tags.clone_from(&before.tags);
            after.notes.clone_from(&before.notes);
        }
        if self.assistant.trial.is_none() {
            self.remember();
        }
        self.assistant.trial = Some(Trial {
            at,
            before,
            after: after.clone(),
            suggested: proposal.preset.blocks.clone(),
            tags: proposal.preset.tags.clone(),
            hearing_after: true,
            trimmed,
            edits_current: proposal.edits_current,
        });
        self.selected = after.blocks.iter().position(|block| block.kind.is_amp());
        self.restore(after, window, cx);
    }

    /// Flips between the suggestion and what came before. Edits made since
    /// trying it stay with their side.
    pub(super) fn compare_trial(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.preset.clone();
        let Some(trial) = &mut self.assistant.trial else {
            return;
        };
        let next = if trial.hearing_after {
            trial.after = current;
            trial.before.clone()
        } else {
            trial.after.clone()
        };
        trial.hearing_after = !trial.hearing_after;
        self.restore(next, window, cx);
    }

    /// Backs out of the suggestion, which counts against tones like it.
    pub(super) fn reject_trial(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(trial) = self.assistant.trial.take() {
            self.assistant.taste.note_rejected(&trial.tags);
            self.save_taste(cx);
            self.history.pop();
            self.restore(trial.before, window, cx);
        }
    }

    /// Saves the suggestion as a tone. A new tone gets a name of its own, and an
    /// edit saves over the tone it came from, like Save does.
    pub(super) fn keep_trial(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(trial) = &self.assistant.trial else {
            return;
        };
        let edits_current = trial.edits_current;
        if !trial.hearing_after {
            self.compare_trial(window, cx);
        }
        let name = self.preset.name.clone();
        let name = if edits_current {
            name
        } else {
            let taken: Vec<&str> = self
                .presets
                .iter()
                .map(|entry| entry.preset.name.as_str())
                .collect();
            unique_name(&name, &taken)
        };
        self.preset_name
            .update(cx, |input, cx| input.set_value(name, window, cx));
        self.save_preset(cx);
    }

    /// Called after a tone saves. If a suggestion was on trial, keeping it is
    /// the strongest thing the player can say about it.
    pub(super) fn trial_saved(&mut self, cx: &mut Context<Self>) {
        if let Some(trial) = self.assistant.trial.take() {
            let kept = self.preset.blocks.clone();
            let taste = &mut self.assistant.taste;
            taste.note_kept(&trial.tags);
            taste.note_adjustments(&trial.suggested, &kept);
            self.save_taste(cx);
        }
    }

    /// Lets go of a trial without judging it, when the rig moves on.
    pub(super) fn drop_trial(&mut self) {
        self.assistant.trial = None;
    }

    /// Opens the TONE3000 picker for Claude. Sign-in comes first if needed, and
    /// the pick comes back through [`Self::tone_picked`].
    pub(super) fn open_picker(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = self.assistant.picker.take() else {
            return;
        };
        self.assistant.awaiting_pick = true;
        let kind = match picker.gear {
            amp_assistant::PickerGear::Amp => BlockKind::Amp,
            amp_assistant::PickerGear::Pedal => BlockKind::Pedal,
            amp_assistant::PickerGear::Ir => BlockKind::Cab,
        };
        self.open_catalog_for(kind, cx);
    }

    /// Whether a sign-in that just came back was on Claude's behalf.
    pub(super) fn awaiting_pick(&mut self) -> bool {
        std::mem::take(&mut self.assistant.awaiting_pick)
    }

    /// Tells Claude what the player picked, so it can build on it.
    pub(super) fn tone_picked(&mut self, tone: &amp_tone3000::Tone, cx: &mut Context<Self>) {
        self.overlay = Overlay::None;
        self.send(
            format!(
                "I picked this on TONE3000: {}, {} {}, by {}. Its tone id is {}. Use it.",
                tone.title,
                tone.gear_label(),
                tone.format_label(),
                tone.creator().username,
                tone.id
            ),
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::unique_name;

    #[test]
    fn a_suggestion_never_takes_an_existing_name() {
        assert_eq!(unique_name("Thaw", &["Moss"]), "Thaw");
        assert_eq!(unique_name("Thaw", &["thaw"]), "Thaw 2");
        assert_eq!(unique_name("Thaw", &["Thaw", "Thaw 2"]), "Thaw 3");
    }
}
