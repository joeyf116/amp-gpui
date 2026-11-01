//! How the assistant panel looks. It docks beside the rig instead of covering
//! it, so the player can keep an eye on the chain and keep playing.

use super::{
    Rig,
    assistant::Page,
    tone3000::{logo, mark},
};
use crate::style::{fog, hairline, moss, moss_glow, pane, sheet};
use amp_assistant::{ModelChoice, Proposal, Role, STARTERS, Turn};
use amp_preset::ToneOrigin;
use amp_tone3000::WEBSITE;
use gpui_kit::{
    component::{
        ActiveTheme, Disableable, Sizable, StyledExt,
        button::{Button, ButtonVariants},
        h_flex,
        input::{Input, Textarea},
        switch::Switch,
        text::TextView,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};

pub(super) const PANEL_WIDTH: f32 = 400.0;
const CONSOLE: &str = "https://platform.claude.com/settings/keys";
const PAGES: [(Page, &str); 4] = [
    (Page::Chat, "Chat"),
    (Page::History, "Chats"),
    (Page::Taste, "Taste"),
    (Page::Settings, "Settings"),
];
const KEY_NOTE: &str = "The assistant runs on Claude through your own Anthropic API key, billed \
                        by Anthropic per request. The key is encrypted to your Windows account \
                        and only ever goes to Anthropic.";
const TASTE_NOTE: &str = "What the assistant has learned about you. It stays on this PC, and \
                          it's sent with your requests so suggestions fit you.";

fn small_note(text: impl Into<SharedString>) -> Div {
    div().text_xs().text_color(fog()).child(text.into())
}

fn heading(text: &'static str) -> Div {
    div()
        .text_xs()
        .font_semibold()
        .text_color(fog())
        .child(text)
}

impl Rig {
    fn render_page_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs: Vec<AnyElement> = PAGES
            .iter()
            .enumerate()
            .map(|(index, (page, title))| {
                let page = *page;
                let current = self.assistant.page == page;
                div()
                    .id(("assistant-page", index))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_sm()
                    .cursor_pointer()
                    .when(current, |tab| tab.bg(moss_glow(0.14)).text_color(moss()))
                    .when(!current, |tab| tab.hover(|tab| tab.bg(moss_glow(0.06))))
                    .child(*title)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.assistant.page = page;
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect();
        h_flex()
            .flex_shrink_0()
            .gap_1()
            .items_center()
            .px_2()
            .py_2()
            .border_b_1()
            .border_color(hairline())
            .children(tabs)
            .child(div().flex_1())
            .child(
                Button::new("assistant-new")
                    .small()
                    .ghost()
                    .label("New")
                    .tooltip("Start a new chat")
                    .on_click(cx.listener(|this, _, _, cx| this.new_chat(cx))),
            )
            .child(
                Button::new("assistant-close")
                    .small()
                    .ghost()
                    .label("Close")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_assistant(window, cx);
                    })),
            )
    }

    /// A tone's credit, which TONE3000's terms require before anything else.
    fn render_credit(&self, id: (usize, usize, usize), credit: &ToneOrigin) -> impl IntoElement {
        let license = credit
            .license
            .as_ref()
            .map(|license| format!(", {license}"))
            .unwrap_or_default();
        h_flex()
            .id(SharedString::from(format!(
                "credit-{}-{}-{}",
                id.0, id.1, id.2
            )))
            .gap_2()
            .items_center()
            .child(self.picture(credit.image.as_deref(), 40.0))
            .child(
                v_flex()
                    .min_w_0()
                    .flex_1()
                    .child(div().text_sm().truncate().child(credit.title.clone()))
                    .child(small_note(format!(
                        "{}  /  {}{license}",
                        credit.gear, credit.format
                    )))
                    .child(small_note(format!("by {}", credit.creator))),
            )
            .child(mark(9.0))
    }

    fn render_trial_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let hearing_after = self
            .assistant
            .trial
            .as_ref()
            .is_some_and(|trial| trial.hearing_after);
        h_flex()
            .gap_1()
            .child(
                Button::new("trial-compare")
                    .small()
                    .outline()
                    .label(if hearing_after {
                        "Hearing new"
                    } else {
                        "Hearing old"
                    })
                    .tooltip("Flip between old and new (Ctrl+B)")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.compare_trial(window, cx);
                    })),
            )
            .child(
                Button::new("trial-keep")
                    .small()
                    .primary()
                    .label("Keep")
                    .on_click(cx.listener(|this, _, window, cx| this.keep_trial(window, cx))),
            )
            .child(
                Button::new("trial-back")
                    .small()
                    .ghost()
                    .label("Back")
                    .on_click(cx.listener(|this, _, window, cx| this.reject_trial(window, cx))),
            )
    }

    fn render_proposal(
        &self,
        at: (usize, usize),
        proposal: &Proposal,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let trial = self.assistant.trial.as_ref().filter(|trial| trial.at == at);
        let trimmed = trial.and_then(|trial| trial.trimmed);
        let on_trial = trial.is_some();
        let preparing = self.assistant.preparing == Some(at);
        let downloads = proposal.downloads();
        let credits: Vec<AnyElement> = proposal
            .credits()
            .enumerate()
            .map(|(index, credit)| {
                self.render_credit((at.0, at.1, index), credit)
                    .into_any_element()
            })
            .collect();
        let has_credits = !credits.is_empty();
        let try_label = match (preparing, downloads) {
            (true, 0) => "Getting it ready".to_owned(),
            (true, _) => "Downloading".to_owned(),
            (false, 0) => "Try it".to_owned(),
            (false, 1) => "Download and try".to_owned(),
            (false, count) => format!("Download {count} and try"),
        };
        v_flex()
            .gap_2()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(if on_trial { moss() } else { hairline() })
            .bg(pane())
            .child(div().font_semibold().child(proposal.preset.name.clone()))
            .when(!proposal.why.is_empty(), |card| {
                card.child(div().text_sm().child(proposal.why.clone()))
            })
            .when(proposal.changes.is_empty(), |card| {
                card.child(small_note(proposal.chain_line()))
            })
            .when(!proposal.changes.is_empty(), |card| {
                card.child(
                    v_flex()
                        .gap(px(2.))
                        .children(proposal.changes.iter().cloned().map(small_note)),
                )
            })
            .children(credits)
            .when(has_credits, |card| {
                card.child(
                    div()
                        .id(SharedString::from(format!("powered-{}-{}", at.0, at.1)))
                        .text_xs()
                        .text_color(fog())
                        .cursor_pointer()
                        .hover(|link| link.text_color(moss()))
                        .child("Powered by TONE3000")
                        .on_click(|_, _, cx| cx.open_url(WEBSITE)),
                )
            })
            .when_some(trimmed, |card, trimmed| {
                card.child(small_note(format!(
                    "Turned down {trimmed:.0} dB to match what you were playing."
                )))
            })
            .map(|card| {
                if on_trial {
                    card.child(self.render_trial_controls(cx))
                } else {
                    card.child(
                        Button::new(SharedString::from(format!("try-{}-{}", at.0, at.1)))
                            .small()
                            .outline()
                            .label(try_label)
                            .disabled(preparing)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.try_proposal(at, window, cx);
                            })),
                    )
                }
            })
    }

    fn render_turn(
        &self,
        index: usize,
        turn: &Turn,
        last: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let proposals: Vec<AnyElement> = turn
            .proposals
            .iter()
            .enumerate()
            .map(|(number, proposal)| {
                self.render_proposal((index, number), proposal, cx)
                    .into_any_element()
            })
            .collect();
        let is_user = turn.role == Role::User;
        let idle = self.assistant.running.is_none();
        let last_user = self
            .assistant
            .chat
            .turns
            .iter()
            .rposition(|turn| turn.role == Role::User)
            == Some(index);
        let text = turn.text.clone();
        v_flex()
            .gap_2()
            .when(is_user, |column| {
                column.child(
                    div()
                        .self_end()
                        .max_w(relative(0.9))
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .bg(moss_glow(0.12))
                        .text_sm()
                        .child(turn.text.clone()),
                )
            })
            .when(!is_user && !turn.text.is_empty(), |column| {
                column.child(
                    TextView::markdown(("reply", index), text.clone())
                        .selectable(true)
                        .text_sm(),
                )
            })
            .children(proposals)
            .children(
                turn.remembered
                    .iter()
                    .map(|fact| small_note(format!("Remembered: {fact}"))),
            )
            .when(is_user && last_user && idle, |column| {
                column.child(
                    h_flex().justify_end().child(
                        Button::new("edit-last")
                            .xsmall()
                            .ghost()
                            .label("Edit")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.edit_last(window, cx);
                            })),
                    ),
                )
            })
            .when(!is_user && !turn.text.is_empty() && idle, |column| {
                column.child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(
                            Button::new(("copy-reply", index))
                                .xsmall()
                                .ghost()
                                .label("Copy")
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                                }),
                        )
                        .when(last, |row| {
                            row.child(
                                Button::new("regenerate")
                                    .xsmall()
                                    .ghost()
                                    .label("Regenerate")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.regenerate(cx);
                                    })),
                            )
                        }),
                )
            })
    }

    fn render_starters(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let starters: Vec<AnyElement> = STARTERS
            .iter()
            .enumerate()
            .map(|(index, starter)| {
                div()
                    .id(("starter", index))
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .border_1()
                    .border_color(hairline())
                    .text_sm()
                    .cursor_pointer()
                    .hover(|row| row.bg(moss_glow(0.06)))
                    .child(*starter)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.send((*starter).to_owned(), cx);
                    }))
                    .into_any_element()
            })
            .collect();
        v_flex()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .text_color(fog())
                    .child("Describe a sound, or say what to change about this one."),
            )
            .children(starters)
    }

    fn render_chat_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let state = &self.assistant;
        let count = state.chat.turns.len();
        let turns: Vec<AnyElement> = state
            .chat
            .turns
            .iter()
            .enumerate()
            .map(|(index, turn)| {
                self.render_turn(index, turn, index + 1 == count, cx)
                    .into_any_element()
            })
            .collect();
        let running = state.running.is_some();
        let picker = state.picker.clone();
        let danger = cx.theme().danger;
        let empty = turns.is_empty();
        v_flex()
            .flex_1()
            .min_h_0()
            .child(
                v_flex()
                    .id("assistant-chat")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&state.scroll)
                    .gap_4()
                    .p_3()
                    .when(empty, |chat| chat.child(self.render_starters(cx)))
                    .children(turns)
                    .when_some(state.status, |chat, status| {
                        chat.child(small_note(format!("{status}...")))
                    })
                    .when_some(picker, |chat, picker| {
                        chat.child(
                            v_flex()
                                .gap_2()
                                .p_3()
                                .rounded_md()
                                .border_1()
                                .border_color(hairline())
                                .child(logo(14.0))
                                .child(
                                    div()
                                        .text_sm()
                                        .child(format!("Look for: {}", picker.looking_for)),
                                )
                                .child(
                                    Button::new("open-picker")
                                        .small()
                                        .outline()
                                        .child(mark(9.0))
                                        .label("Browse TONE3000")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.open_picker(cx);
                                        })),
                                ),
                        )
                    })
                    .when_some(state.failure.clone(), |chat, failure| {
                        chat.child(
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(danger)
                                        .child(failure.to_string()),
                                )
                                .child(
                                    Button::new("try-again")
                                        .small()
                                        .ghost()
                                        .label("Try again")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.regenerate(cx);
                                        })),
                                ),
                        )
                    })
                    .when_some(state.notice.clone(), |chat, notice| {
                        chat.child(small_note(notice))
                    }),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .gap_2()
                    .items_end()
                    .p_2()
                    .border_t_1()
                    .border_color(hairline())
                    .child(div().flex_1().min_w_0().child(Textarea::new(&state.input)))
                    .map(|row| {
                        if running {
                            row.child(
                                Button::new("stop-reply")
                                    .small()
                                    .outline()
                                    .label("Stop")
                                    .on_click(cx.listener(|this, _, _, cx| this.stop_reply(cx))),
                            )
                        } else {
                            row.child(
                                Button::new("send")
                                    .small()
                                    .primary()
                                    .label("Send")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.send_from_input(window, cx);
                                    })),
                            )
                        }
                    }),
            )
    }

    fn render_history_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.assistant.chat.id;
        let renaming = self.assistant.renaming.clone();
        let rows: Vec<AnyElement> = self
            .assistant
            .chats
            .iter()
            .enumerate()
            .map(|(index, chat)| {
                let id = chat.id;
                let is_current = id == current;
                let title = if chat.title.is_empty() {
                    "Untitled chat".to_owned()
                } else {
                    chat.title.clone()
                };
                h_flex()
                    .id(("saved-chat", index))
                    .gap_2()
                    .items_center()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .when(is_current, |row| row.bg(moss_glow(0.12)))
                    .when(!is_current, |row| row.hover(|row| row.bg(moss_glow(0.06))))
                    .map(|row| match (&renaming, is_current) {
                        (Some(input), true) => {
                            row.child(div().flex_1().child(Input::new(input).small()))
                        }
                        _ => row.child(div().flex_1().min_w_0().text_sm().truncate().child(title)),
                    })
                    .when(is_current && renaming.is_none(), |row| {
                        row.child(
                            Button::new("rename-chat")
                                .xsmall()
                                .ghost()
                                .label("Rename")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.start_rename(window, cx);
                                })),
                        )
                    })
                    .child(
                        Button::new(("delete-chat", index))
                            .xsmall()
                            .ghost()
                            .label("Delete")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.delete_chat(id, cx);
                            })),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.open_chat(id, cx)))
                    .into_any_element()
            })
            .collect();
        let empty = rows.is_empty();
        v_flex()
            .id("assistant-history")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .gap_px()
            .p_2()
            .when(empty, |list| {
                list.child(
                    div()
                        .p_2()
                        .child(small_note("Your chats will show up here.")),
                )
            })
            .children(rows)
    }

    fn render_taste_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let taste = &self.assistant.taste;
        let facts: Vec<AnyElement> = taste
            .facts
            .iter()
            .enumerate()
            .map(|(index, fact)| {
                h_flex()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .child(div().text_sm().child(fact.text.clone()))
                    .child(
                        Button::new(("forget-fact", index))
                            .xsmall()
                            .ghost()
                            .label("Forget")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.forget_fact(index, cx);
                            })),
                    )
                    .into_any_element()
            })
            .collect();
        let signals = taste.signal_lines();
        let nothing = facts.is_empty() && signals.is_empty();
        v_flex()
            .id("assistant-taste")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .gap_3()
            .p_3()
            .child(small_note(TASTE_NOTE))
            .child(
                Switch::new("learning")
                    .checked(taste.learning)
                    .label("Learn from what I keep and adjust")
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_taste_learning(cx))),
            )
            .when(nothing, |page| {
                page.child(small_note(
                    "Nothing yet. Tell it about your guitar, or keep a few suggestions.",
                ))
            })
            .when(!facts.is_empty(), |page| {
                page.child(heading("What you've said")).children(facts)
            })
            .when(!signals.is_empty(), |page| {
                page.child(heading("What it's noticed"))
                    .children(signals.into_iter().map(|line| div().text_sm().child(line)))
            })
            .when(!nothing, |page| {
                page.child(
                    Button::new("clear-taste")
                        .small()
                        .ghost()
                        .label("Forget everything")
                        .on_click(cx.listener(|this, _, _, cx| this.clear_taste(cx))),
                )
            })
    }

    fn render_key_entry(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let checking = self.assistant.checking_key;
        v_flex()
            .gap_2()
            .child(small_note(KEY_NOTE))
            .child(
                Button::new("open-console")
                    .small()
                    .ghost()
                    .label("Get a key from the Claude Console")
                    .on_click(|_, _, cx| cx.open_url(CONSOLE)),
            )
            .child(Input::new(&self.assistant.key_input).mask_toggle().small())
            .child(
                Button::new("save-key")
                    .small()
                    .primary()
                    .label(if checking { "Checking" } else { "Save key" })
                    .disabled(checking)
                    .on_click(cx.listener(|this, _, window, cx| this.save_key(window, cx))),
            )
    }

    fn render_settings_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let state = &self.assistant;
        let model = self.settings.assistant_model;
        let cost = model.cost(state.usage);
        let models: Vec<AnyElement> = ModelChoice::ALL
            .iter()
            .enumerate()
            .map(|(index, choice)| {
                let choice = *choice;
                let current = choice == model;
                div()
                    .id(("model-choice", index))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_sm()
                    .cursor_pointer()
                    .when(current, |row| row.bg(moss_glow(0.14)).text_color(moss()))
                    .when(!current, |row| row.hover(|row| row.bg(moss_glow(0.06))))
                    .child(choice.title())
                    .on_click(cx.listener(move |this, _, _, cx| this.set_model(choice, cx)))
                    .into_any_element()
            })
            .collect();
        let has_key = state.key.is_some();
        v_flex()
            .id("assistant-settings")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .gap_3()
            .p_3()
            .child(heading("API key"))
            .when(has_key, |page| {
                page.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().flex_1().text_sm().child("Saved"))
                        .child(
                            Button::new("remove-key")
                                .small()
                                .ghost()
                                .label("Remove key")
                                .on_click(cx.listener(|this, _, _, cx| this.remove_key(cx))),
                        ),
                )
            })
            .when(!has_key, |page| page.child(self.render_key_entry(cx)))
            .when_some(state.notice.clone(), |page, notice| {
                page.child(div().text_sm().text_color(cx.theme().warning).child(notice))
            })
            .child(heading("Model"))
            .child(v_flex().gap_px().children(models))
            .child(small_note(
                "Opus thinks hardest. Haiku costs the least and answers fastest.",
            ))
            .child(heading("This session"))
            .child(small_note(format!(
                "{} tokens in, {} out, about ${cost:.3}",
                state.usage.input_tokens
                    + state.usage.cache_read_input_tokens
                    + state.usage.cache_creation_input_tokens,
                state.usage.output_tokens
            )))
    }

    pub(super) fn render_assistant(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let needs_key = self.assistant.key.is_none();
        let page = if needs_key && self.assistant.page == Page::Chat {
            Page::Settings
        } else {
            self.assistant.page
        };
        v_flex()
            .id("assistant")
            .occlude()
            .flex_shrink_0()
            .w(px(PANEL_WIDTH))
            .h_full()
            .border_l_1()
            .border_color(hairline())
            .bg(sheet())
            .child(self.render_page_tabs(cx))
            .map(|panel| match page {
                Page::Chat => panel.child(self.render_chat_page(cx)),
                Page::History => panel.child(self.render_history_page(cx)),
                Page::Taste => panel.child(self.render_taste_page(cx)),
                Page::Settings => panel.child(self.render_settings_page(cx)),
            })
    }
}
