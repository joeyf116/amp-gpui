//! Tones on the rig: undo, loading, saving, and the files behind them.

use super::{ENGINE_RATE, HISTORY, Rig};
use amp_dsp::BlockKind;
use amp_engine::Command;
use amp_preset::{AssetKind, Library, LibraryError, Preset, build_chain};
use gpui_kit::*;
use std::path::PathBuf;

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
        // Stay on the unit being worked on, if it's still there.
        self.selected = self.selected.filter(|slot| *slot < preset.blocks.len());
        self.saved_blocks = self
            .presets
            .iter()
            .find(|entry| entry.preset.name == preset.name)
            .map_or_else(Vec::new, |entry| entry.preset.blocks.clone());
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
        self.want_block_images(cx);
        self.error = None;
        if let Some(engine) = &mut self.engine {
            engine.send(Command::SetChain(Box::new(built.chain)));
            // Knobs may have moved while the chain was building.
            for (slot, block) in self.preset.blocks.iter().enumerate() {
                for param in 0..block.kind.params().len() {
                    engine.send(Command::SetParam {
                        slot,
                        param,
                        value: block.param(param),
                    });
                }
            }
        }
        cx.notify();
    }

    /// Builds `preset` in the background and only adopts it if every model
    /// loads, so a bad file leaves the tone playing as it was.
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
                        if !std::mem::take(&mut this.starting)
                            && this.preset.blocks != preset.blocks
                        {
                            this.remember();
                        }
                        this.drop_trial();
                        this.saved_blocks = preset.blocks.clone();
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

    pub(super) fn save_preset(&mut self, cx: &mut Context<Self>) {
        self.preset.name = self.preset_name.read(cx).value().to_string();
        match self.library.save_preset(&self.preset) {
            Ok(_) => {
                self.saved_blocks = self.preset.blocks.clone();
                self.settings.last_tone = Some(self.preset.name.clone());
                self.save_settings(cx);
                self.refresh_presets();
                self.trial_saved(cx);
            }
            Err(error) => self.set_error(error, cx),
        }
        cx.notify();
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
