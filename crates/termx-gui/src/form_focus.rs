//! Automaticky fokus na prvni pole formulare/dialogu pri jeho otevreni
//! (zpetna vazba "když je nějaký formulář tak bychom měli mít fokus na
//! prvním řádku formuláře" - napr. "Přesunout do složky" nebo
//! "Přejmenovat" se otevrely bez fokusu a muselo se nejdriv kliknout).
//!
//! Na rozdil od starsiho vzoru s priznakem `focus_requested` ve stavu
//! kazdeho dialogu (`NewSessionForm`, `NewFolderDialog`...) tohle
//! nepotrebuje zadne pole navic: "dialog se prave otevrel" se pozna tak,
//! ze pole NEBYLO vykresleno v bezprostredne predchozim pruchodu UI
//! (`Context::cumulative_pass_nr`, cislo posledniho vykresleni se drzi v
//! docasne pameti egui pod ID pole). Funguje tak i pro dialogy, jejichz
//! stav je jen `Option<String>` (SFTP/FTP "Nová složka"/"Přejmenovat"), a
//! pro prihlasovaci formular v tabu - ten dostane fokus pokazde, kdyz se
//! na tab prepne.
//!
//! Pohyb mezi poli Tabem (a Shift+Tabem) resi egui samo.

use egui::text::{CCursor, CCursorRange};
use egui::widgets::text_edit::TextEditState;

/// Da fokus `resp` (textove pole), pokud se v tomto pruchodu UI vykresluje
/// poprve (dialog/formular se prave objevil). Vraci `true`, pokud fokus
/// pozadoval.
pub(crate) fn autofocus_on_open(resp: &egui::Response) -> bool {
    let ctx = &resp.ctx;
    let key = resp.id.with("termix_autofocus_pass");
    let now = ctx.cumulative_pass_nr();
    let last: Option<u64> = ctx.data(|d| d.get_temp(key));
    ctx.data_mut(|d| d.insert_temp(key, now));
    // `last == now` = stejne pole vykreslene vicekrat v jednom pruchodu,
    // `last + 1 == now` = bylo videt uz minule -> nic.
    let just_opened = last.map_or(true, |l| l.saturating_add(1) < now);
    if just_opened {
        resp.request_focus();
    }
    just_opened
}

/// Jako [`autofocus_on_open`], navic oznaci cely text pole - pro
/// "Přejmenovat": puvodni nazev jde rovnou prepsat psanim, nebo sipkou
/// upravit.
pub(crate) fn autofocus_select_all(resp: &egui::Response, text: &str) {
    if autofocus_on_open(resp) {
        let mut state = TextEditState::load(&resp.ctx, resp.id).unwrap_or_default();
        let len = text.chars().count();
        state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(len))));
        state.store(&resp.ctx, resp.id);
    }
}

/// Enter v jednopolovem dialogu = potvrdit (stejny vzor jako uz drive
/// "Nová složka" v `app.rs`): egui pri Enteru v singleline poli zrusi jeho
/// fokus, takze `lost_focus()` + stisknuty Enter.
pub(crate) fn enter_pressed(resp: &egui::Response) -> bool {
    resp.lost_focus() && resp.ctx.input(|i| i.key_pressed(egui::Key::Enter))
}
