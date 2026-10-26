//! Encrypted file tabs - a file whose name says it is kept encrypted with gpg (see
//! [`crate::api::encrypted_files`]): the passphrase asked for before its text is shown, and
//! again, typed twice, before every save.
//!
//! The tab never keeps a passphrase. What is typed lives in a buffer that is wiped as it is
//! let go of - see [`super::passphrase_field`] - and only for as long as the decrypt or the
//! save it was typed for is out. The text is never written in the clear: the repo's side
//! refuses a plain write of such a name, and the tab tells no language server about it.

use egui::{Key, RichText, Ui};
use egui_frames::PaneId;
use zeroize::Zeroizing;

use crate::{
    api::encrypted_files::DecryptFileRequest,
    native::{
        app::App,
        theme::{Palette, SMALL_SIZE},
        widgets,
    },
};

use super::passphrase_field::{empty_passphrase, passphrase_field, wipe};

/// The passphrase question of an encrypted tab, when one is up.
pub(super) struct Encrypted {
    asking: Option<PassphraseAsk>,
}

/// What a passphrase is being asked for.
#[derive(Clone, Copy, PartialEq)]
enum AskingFor {
    /// To decrypt the file, which shows nothing until it is.
    Opening,
    /// To encrypt the text for a save. Typed twice, since a typo here is a file nobody can
    /// open.
    Saving,
}

struct PassphraseAsk {
    asking_for: AskingFor,
    typed: Zeroizing<String>,
    repeated: Zeroizing<String>,
    /// Why the last try did not go through - a wrong passphrase, two that differ.
    refused: Option<String>,
    /// Whether a decrypt with what was typed is out.
    trying: bool,
    /// Set as the question is put, so the first field takes the keyboard once.
    wants_focus: bool,
}

impl PassphraseAsk {
    fn new(asking_for: AskingFor) -> Self {
        Self {
            asking_for,
            typed: empty_passphrase(),
            repeated: empty_passphrase(),
            refused: None,
            trying: false,
            wants_focus: true,
        }
    }
}

impl Encrypted {
    /// A file that is on disk: asks for its passphrase before anything is shown.
    pub(super) fn opening() -> Self {
        Self {
            asking: Some(PassphraseAsk::new(AskingFor::Opening)),
        }
    }

    /// A file nothing is at yet: written into freely, asked a passphrase as it is saved.
    pub(super) fn new_file() -> Self {
        Self { asking: None }
    }

    /// Put the question a save asks up. Answering it is what writes the file - see
    /// [`App::write_file_pane`].
    pub(super) fn ask_to_save(&mut self) {
        if self.asking.is_none() {
            self.asking = Some(PassphraseAsk::new(AskingFor::Saving));
        }
    }

    /// Whether a passphrase is being asked for.
    pub(super) fn is_asking(&self) -> bool {
        self.asking.is_some()
    }

    /// Whether the text is held back behind the question: a file not decrypted yet.
    pub(super) fn hides_the_text(&self) -> bool {
        self.asking
            .as_ref()
            .is_some_and(|ask| ask.asking_for == AskingFor::Opening)
    }
}

#[cfg(test)]
impl Encrypted {
    pub(super) fn refusal_for_test(&self) -> Option<String> {
        self.asking.as_ref().and_then(|ask| ask.refused.clone())
    }
}

/// What a press in the question asked for, acted on once the editor is no longer borrowed.
enum Answered {
    Open(Zeroizing<String>),
    Save(Zeroizing<String>),
    Cancel,
}

impl App {
    /// The passphrase question of an encrypted tab, when it has one up.
    pub(super) fn draw_passphrase_ask(
        &mut self,
        ui: &mut Ui,
        pane_id: PaneId,
        session_id: &str,
        palette: &Palette,
    ) {
        let Some(ask) = self
            .model
            .file_editors
            .get_mut(&pane_id)
            .and_then(|editor| editor.encrypted.as_mut())
            .and_then(|encrypted| encrypted.asking.as_mut())
        else {
            return;
        };
        let mut answered = None;
        ui.add_space(8.0);
        ui.label(match ask.asking_for {
            AskingFor::Opening => "This file is encrypted. Type its passphrase to open it.",
            AskingFor::Saving => "Type the passphrase to encrypt this file with, twice.",
        });
        ui.add_space(4.0);
        let (typed, mut submitted) = passphrase_field(ui, &mut ask.typed, "passphrase");
        if std::mem::take(&mut ask.wants_focus) {
            typed.request_focus();
        }
        if ask.asking_for == AskingFor::Saving {
            ui.add_space(4.0);
            let (_, entered) = passphrase_field(ui, &mut ask.repeated, "passphrase again");
            submitted |= entered;
        }
        ui.horizontal(|ui| match ask.asking_for {
            AskingFor::Opening => {
                if ask.trying {
                    ui.spinner();
                } else if widgets::quiet_button(ui, "[open]").clicked() {
                    submitted = true;
                }
            }
            AskingFor::Saving => {
                if widgets::quiet_button(ui, "[encrypt and save]").clicked() {
                    submitted = true;
                }
                if widgets::quiet_button(ui, "[cancel]").clicked()
                    || ui.input(|input| input.key_pressed(Key::Escape))
                {
                    answered = Some(Answered::Cancel);
                }
            }
        });
        if let Some(refused) = &ask.refused {
            ui.label(
                RichText::new(refused)
                    .size(SMALL_SIZE - 1.0)
                    .color(palette.warn),
            );
        }
        if submitted && !ask.trying && answered.is_none() {
            answered = match ask.asking_for {
                AskingFor::Opening if ask.typed.is_empty() => None,
                AskingFor::Opening => Some(Answered::Open(ask.typed.clone())),
                AskingFor::Saving if ask.typed.is_empty() => {
                    ask.refused = Some("type a passphrase".to_string());
                    None
                }
                AskingFor::Saving if ask.typed != ask.repeated => {
                    ask.refused = Some("the two passphrases differ".to_string());
                    wipe(&mut ask.repeated);
                    None
                }
                AskingFor::Saving => Some(Answered::Save(ask.typed.clone())),
            };
        }

        match answered {
            None => {}
            Some(Answered::Open(passphrase)) => self.decrypt_file(pane_id, session_id, passphrase),
            Some(Answered::Save(passphrase)) => {
                // Dropping the question wipes both fields.
                self.encrypted_mut(pane_id).asking = None;
                self.write_file_pane(pane_id, session_id, Some(passphrase));
            }
            Some(Answered::Cancel) => self.encrypted_mut(pane_id).asking = None,
        }
    }

    fn encrypted_mut(&mut self, pane_id: PaneId) -> &mut Encrypted {
        self.model
            .file_editors
            .get_mut(&pane_id)
            .and_then(|editor| editor.encrypted.as_mut())
            .expect("an encrypted tab has its encryption")
    }

    /// Read the file with what was typed: its text goes on screen, or the question stays up
    /// saying why not. The passphrase is wiped either way once the decrypt is over.
    fn decrypt_file(&mut self, pane_id: PaneId, session_id: &str, passphrase: Zeroizing<String>) {
        let ask = self
            .encrypted_mut(pane_id)
            .asking
            .as_mut()
            .expect("a decrypt is asked for from the question");
        ask.trying = true;
        ask.refused = None;
        let file_path = self.model.file_editors[&pane_id].file_path.clone();
        let for_call = session_id.to_string();
        let request = DecryptFileRequest {
            file_path,
            passphrase,
        };
        self.tasks.spawn_keyed(
            Some(format!("file:{pane_id}")),
            move |backend| backend.decrypted_file_content(&for_call, &request),
            move |model, result| {
                let Some(editor) = model.file_editors.get_mut(&pane_id) else {
                    return;
                };
                let encrypted = editor
                    .encrypted
                    .as_mut()
                    .expect("an encrypted tab has its encryption");
                match result {
                    Ok(payload) => {
                        encrypted.asking = None;
                        editor.take_what_is_on_disk(payload);
                    }
                    Err(error) => {
                        let ask = encrypted
                            .asking
                            .as_mut()
                            .expect("the question stays up while its decrypt is out");
                        ask.trying = false;
                        wipe(&mut ask.typed);
                        ask.wants_focus = true;
                        ask.refused = Some(format!("{error}"));
                    }
                }
            },
        );
    }
}
