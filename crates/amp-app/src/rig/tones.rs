//! Tones on the rig: undo, loading, saving, and the files behind them.

use super::{ENGINE_RATE, HISTORY, Rig};
use amp_dsp::BlockKind;
use amp_engine::Command;
use amp_preset::{
    AssetKind, Library, LibraryError, Preset, build_chain, level_slot, matched_level,
};
use gpui_kit::*;
use std::path::PathBuf;

/// `name`, or the first free name after it, such as `Crunch 2`. Saving as new
/// and keeping a suggestion never replace a tone the player already has.
pub(super) fn unique_name(name: &str, taken: &[&str]) -> String {
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

impl Rig {
    /// Keeps the board as it is now, for Undo.
    pub(super) fn remember(&mut self) {
        self.history.push(self.preset.clone());
        if self.history.len() > HISTORY {
            self.history.remove(0);
        }
        self.future.clear();
        self.last_param = None;
    }

    /// Puts `preset` on the rig as it stands, saved or not. The asterisk in the
    /// header follows from what the library holds under its name.
    pub(super) fn restore(&mut self, preset: Preset, window: &mut Window, cx: &mut Context<Self>) {
        self.captures = vec![None; preset.blocks.len()];
        self.problems = vec![None; preset.blocks.len()];
        // Stay on the unit being worked on, if it's still there.
        self.selected = self.selected.filter(|slot| *slot < preset.blocks.len());
        self.saved = self
            .presets
            .iter()
            .find(|entry| entry.preset.name == preset.name)
            .map(|entry| entry.preset.clone());
        self.preset_name.update(cx, |input, cx| {
            input.set_value(preset.name.clone(), window, cx);
        });
        self.preset = preset;
        self.last_param = None;
        self.rebuild_chain(cx);
    }

    pub(super) fn undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(preset) = self.history.pop() {
            self.drop_trial();
            self.future
                .push(std::mem::replace(&mut self.preset, preset.clone()));
            self.restore(preset, window, cx);
        }
    }

    pub(super) fn redo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(preset) = self.future.pop() {
            self.drop_trial();
            self.history
                .push(std::mem::replace(&mut self.preset, preset.clone()));
            self.restore(preset, window, cx);
        }
    }

    pub(super) fn step_tone(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.presets.len() as isize;
        if count == 0 {
            return;
        }
        let current = self
            .presets
            .iter()
            .position(|entry| entry.preset.name == self.preset.name)
            .map_or(0, |index| index as isize);
        let next = (current + step).rem_euclid(count) as usize;
        let preset = self.presets[next].preset.clone();
        self.load_preset(preset, window, cx);
    }

    /// Rebuilds the running chain in the background after a board edit. Only
    /// the newest build counts, so quick drags can't arrive out of order.
    pub(super) fn rebuild_chain(&mut self, cx: &mut Context<Self>) {
        self.build += 1;
        let build = self.build;
        let preset = self.preset.clone();
        let library = self.library.clone();
        cx.spawn(async move |this, cx| {
            let task = cx
                .background_executor()
                .spawn(async move { build_chain(&preset, &library, ENGINE_RATE) });
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.build != build {
                    return;
                }
                match result {
                    Ok(built) => this.adopt_chain(built, cx),
                    Err(error) => this.set_error(error, cx),
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn adopt_chain(&mut self, built: amp_preset::BuiltChain, cx: &mut Context<Self>) {
        self.captures = built.captures;
        self.problems = built
            .problems
            .into_iter()
            .map(|problem| problem.map(SharedString::from))
            .collect();
        self.want_block_images(cx);
        self.error = None;
        if let Some(engine) = &mut self.engine {
            engine.send(Command::SetChain(Box::new(built.chain)));
        }
        // Knobs may have moved, or the scene changed, while the chain was building.
        self.sync_settings();
        cx.notify();
    }

    /// Sends every switch, knob, and the Level to the running chain. That's
    /// all a scene changes, so this is a whole scene switch, with no gap and
    /// no reload, and delay and reverb tails ring on through it.
    pub(super) fn sync_settings(&mut self) {
        let Some(engine) = &mut self.engine else {
            return;
        };
        for (slot, block) in self.preset.blocks.iter().enumerate() {
            engine.send(Command::SetBypassed {
                slot,
                bypassed: !block.enabled,
            });
            for param in 0..block.kind.params().len() {
                engine.send(Command::SetParam {
                    slot,
                    param,
                    value: block.param(param),
                });
            }
        }
        engine.send(Command::SetParam {
            slot: level_slot(&self.preset),
            param: 0,
            value: self.preset.level(),
        });
    }

    /// Whether the tone differs from what the library holds under its name.
    pub(super) fn edited(&self) -> bool {
        self.saved
            .as_ref()
            .is_none_or(|saved| !saved.sounds_like(&self.preset))
    }

    /// Plays scene `index` of the tone. Not an edit, so not a step of undo.
    pub(super) fn select_scene(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.preset.switch_scene(index) {
            self.last_param = None;
            self.sync_settings();
            cx.notify();
        }
    }

    /// Adds a scene, copied from the one playing, and moves to it.
    pub(super) fn add_scene(&mut self, cx: &mut Context<Self>) {
        let before = self.preset.clone();
        if self.preset.add_scene().is_some() {
            self.history.push(before);
            self.future.clear();
            self.last_param = None;
            self.sync_settings();
            cx.notify();
        }
    }

    /// Removes the scene that's playing.
    pub(super) fn remove_scene(&mut self, cx: &mut Context<Self>) {
        if self.preset.scenes.is_empty() {
            return;
        }
        self.remember();
        self.preset.remove_scene();
        self.sync_settings();
        cx.notify();
    }

    /// Sets the Level of the tone, or of the scene that's playing.
    pub(super) fn set_tone_level(&mut self, value: f32, cx: &mut Context<Self>) {
        const LEVEL: (usize, usize) = (usize::MAX, 0);
        if self.last_param != Some(LEVEL) {
            self.remember();
            self.last_param = Some(LEVEL);
        }
        self.preset.level_db = value;
        let slot = level_slot(&self.preset);
        if let Some(engine) = &mut self.engine {
            engine.send(Command::SetParam {
                slot,
                param: 0,
                value: self.preset.level(),
            });
        }
        cx.notify();
    }

    /// Plays the tone on a test chord and sets its Level so it sits with the
    /// factory tones. Runs in the background, since it loads every model.
    pub(super) fn match_level(&mut self, cx: &mut Context<Self>) {
        let preset = self.preset.clone();
        let library = self.library.clone();
        let build = self.build;
        let scene = self.preset.scene;
        self.matching_level = true;
        cx.spawn(async move |this, cx| {
            let task = cx
                .background_executor()
                .spawn(async move { matched_level(&preset, &library, ENGINE_RATE) });
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.matching_level = false;
                // A tone that changed under the measurement, or a scene
                // switched to meanwhile, keeps its Level.
                if this.build != build || this.preset.scene != scene {
                    return cx.notify();
                }
                match result {
                    Ok(level) => {
                        this.last_param = None;
                        this.set_tone_level(level, cx);
                        this.last_param = None;
                    }
                    Err(error) => this.set_error(error, cx),
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// Builds `preset` in the background and puts it on the rig. A block whose
    /// file is missing or won't run plays as a pass-through, and its unit says
    /// why, so one bad file never costs the whole tone.
    pub(super) fn load_preset(
        &mut self,
        preset: Preset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.restore_tone_models(&preset, window, cx) {
            return;
        }
        self.build += 1;
        let build = self.build;
        let library = self.library.clone();
        cx.spawn_in(window, async move |this, cx| {
            let task = cx.background_executor().spawn(async move {
                let built = build_chain(&preset, &library, ENGINE_RATE)?;
                Ok::<_, LibraryError>((preset, built))
            });
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.build != build {
                    return;
                }
                match result {
                    Ok((preset, built)) => {
                        this.preset_name.update(cx, |input, cx| {
                            input.set_value(preset.name.clone(), window, cx);
                        });
                        this.settings.last_tone = Some(preset.name.clone());
                        this.note_recent(&preset.name);
                        this.save_settings(cx);
                        // Moving to another tone can be undone like any edit, so
                        // unsaved work on this one is never lost.
                        if !std::mem::take(&mut this.starting) && !this.preset.sounds_like(&preset)
                        {
                            this.remember();
                        }
                        this.drop_trial();
                        this.saved = Some(preset.clone());
                        this.last_param = None;
                        // Open on the amp, the first thing most people reach for.
                        this.selected = preset.blocks.iter().position(|block| {
                            matches!(block.kind, BlockKind::Amp | BlockKind::Tube)
                        });
                        this.preset = preset;
                        this.adopt_chain(built, cx);
                    }
                    Err(error) => this.set_error(error, cx),
                }
            });
        })
        .detach();
    }

    pub(super) fn pick_path(
        &self,
        directories: bool,
        prompt: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, PathBuf, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: !directories,
            directories,
            multiple: false,
            prompt: Some(prompt.into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = paths.await
                && let Some(path) = paths.pop()
            {
                let _ = this.update_in(cx, |this, window, cx| then(this, path, window, cx));
            }
        })
        .detach();
    }

    /// Adds `.nam` files to the rack. One bad file shouldn't stop the rest of a
    /// batch, so the first error is shown after everything else is in.
    pub(super) fn add_captures(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Add captures".into()),
        });
        let library = self.library.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let task = cx.background_executor().spawn(async move {
                let mut first_error = None;
                for path in paths {
                    if let Err(error) = library.add_capture(&path) {
                        first_error.get_or_insert(error.to_string());
                    }
                }
                first_error
            });
            let error = task.await;
            let _ = this.update(cx, |this, cx| {
                this.refresh_catalog();
                if let Some(error) = error {
                    this.error = Some(error.into());
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Points an amp or pedal unit at a `.nam` file, for a capture that's
    /// missing from the library. The file joins the gear picker too.
    pub(super) fn locate_capture(
        &mut self,
        slot: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Use this capture".into()),
        });
        let library = self.library.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(mut paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.pop() else {
                return;
            };
            let task = cx
                .background_executor()
                .spawn(async move { library.add_capture(&path) });
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.refresh_catalog();
                match result {
                    Ok(entry) if slot < this.preset.blocks.len() => {
                        this.remember();
                        let block = &mut this.preset.blocks[slot];
                        block.asset = Some(entry.asset);
                        block.origin = entry.origin;
                        this.rebuild_chain(cx);
                    }
                    Ok(_) => {}
                    Err(error) => this.set_error(error, cx),
                }
            });
        })
        .detach();
    }

    pub(super) fn load_cab_ir(&mut self, slot: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.pick_path(
            false,
            "Load",
            window,
            cx,
            move |this, path, _, cx| match this
                .library
                .import_asset(AssetKind::ImpulseResponse, &path)
            {
                Ok(asset) => {
                    this.remember();
                    this.preset.blocks[slot].asset = Some(asset);
                    this.rebuild_chain(cx);
                }
                Err(error) => this.set_error(error, cx),
            },
        );
    }

    /// Saves the tone under its own name, replacing what the library holds.
    pub(super) fn save_preset(&mut self, cx: &mut Context<Self>) {
        self.preset.store_scene();
        match self.library.save_preset(&self.preset) {
            Ok(_) => {
                self.saved = Some(self.preset.clone());
                self.settings.last_tone = Some(self.preset.name.clone());
                self.save_settings(cx);
                self.refresh_presets();
                self.trial_saved(cx);
            }
            Err(error) => self.set_error(error, cx),
        }
        cx.notify();
    }

    /// Saves the tone as a new one under the name typed in the tone browser.
    /// It never replaces another tone: a name that's taken gets a number.
    pub(super) fn save_as_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.preset_name.read(cx).value().trim().to_owned();
        let typed = if typed.is_empty() {
            self.preset.name.clone()
        } else {
            typed
        };
        let taken: Vec<&str> = self
            .presets
            .iter()
            .map(|entry| entry.preset.name.as_str())
            .collect();
        let name = unique_name(&typed, &taken);
        self.preset_name
            .update(cx, |input, cx| input.set_value(name.clone(), window, cx));
        self.preset.name = name;
        self.save_preset(cx);
    }

    /// Goes back to the tone as it was last saved. Undo can bring the edits back.
    pub(super) fn revert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(saved) = self.saved.clone() else {
            return;
        };
        if saved.sounds_like(&self.preset) {
            return;
        }
        self.drop_trial();
        self.remember();
        self.restore(saved, window, cx);
    }

    pub(super) fn refresh_presets(&mut self) {
        match self.library.presets() {
            Ok((entries, errors)) => {
                self.presets = entries;
                if let Some(error) = errors.first() {
                    self.error = Some(error.to_string().into());
                }
            }
            Err(error) => self.error = Some(error.to_string().into()),
        }
    }

    pub(super) fn refresh_catalog(&mut self) {
        match self.library.captures() {
            Ok(catalog) => self.catalog = catalog,
            Err(error) => self.error = Some(error.to_string().into()),
        }
    }

    pub(super) fn export_pack(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pick_path(true, "Export here", window, cx, |this, path, _, cx| {
            if let Err(error) = this.library.export_pack(&this.preset, &path) {
                this.set_error(error, cx);
            }
        });
    }

    pub(super) fn import_pack(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pick_path(true, "Import", window, cx, |this, path, _, cx| {
            match this.library.import_pack(&path) {
                Ok(_) => this.refresh_presets(),
                Err(error) => this.error = Some(error.to_string().into()),
            }
            cx.notify();
        });
    }

    pub(super) fn choose_library(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pick_path(true, "Use this folder", window, cx, |this, path, _, cx| {
            match Library::open(path.clone()) {
                Ok(library) => {
                    this.library = library;
                    this.settings.library = Some(path);
                    this.refresh_presets();
                    this.refresh_catalog();
                    this.save_settings(cx);
                }
                Err(error) => this.error = Some(error.to_string().into()),
            }
            cx.notify();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::unique_name;

    #[test]
    fn a_new_tone_never_takes_an_existing_name() {
        assert_eq!(unique_name("Thaw", &["Moss"]), "Thaw");
        assert_eq!(unique_name("Thaw", &["thaw"]), "Thaw 2");
        assert_eq!(unique_name("Thaw", &["Thaw", "Thaw 2"]), "Thaw 3");
    }
}
