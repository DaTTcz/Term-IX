//! Lokalizace UI textu ("i18n") - viz pozadavek "do nastavení bych dal
//! možnost dropdown vybrat si jazyk ve kterém bude aplikace fungovat.
//! Zatím bych dal češtinu a angličtinu."
//!
//! Zamerne NE stringly-keyovany prekladovy katalog (napr. `HashMap<&str,
//! &str>` nebo `match` nad enum klici) - vsechny prekladatelne texty
//! jsou pojmenovana pole struktury [`Strings`], vyplnena jednou pro
//! kazdy jazyk (konstanty [`CS`]/[`EN`]). Vyhoda: kdyz nekdo pozdeji
//! prida nove pole, kompilator SAM nahlasi chybu u obou konstant, dokud
//! nebude preklad vyplnen pro OBA jazyky - na rozdil od stringly-keyovaneho
//! reseni, kde chybejici preklad typicky tise spadne na placeholder nebo
//! na jeden vychozi jazyk, aniz by si toho nekdo vsiml pred behem
//! aplikace.
//!
//! ZAMERNE NEPREKLADANO (zustava vzdy stejne, v obou jazycich):
//! - Nazev aplikace "Term-IX" (vlastni jmeno).
//! - Prihlasovaci "prompt", ktery `termx-gui::terminal` rucne "doopisuje"
//!   primo do bufferu terminalu, aby napodobil chovani skutecneho `ssh`
//!   klienta ("login as: ", "Password: ", "Permission denied, please
//!   try again.") - tohle NENI text aplikace, ale emulace skutecneho
//!   protokoloveho promptu, ktery i anglicky mluvici uzivatel skutecneho
//!   `ssh` klienta vidi vzdy anglicky bez ohledu na system/locale.

use serde::{Deserialize, Serialize};

/// Jazyk UI aplikace - viz [`crate::app::AppSettings::lang`] (uklada se
/// stejne jako ostatni `AppSettings`, preziva restart aplikace) a
/// dropdown v `MainApp::render_settings`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lang {
    Cs,
    En,
}

impl Default for Lang {
    /// Puvodni (a zatim jedine) UI aplikace bylo cesky - zmena chovani
    /// pro existujici uzivatele by byla nemile prekvapeni, takze
    /// vychozi jazyk zustava cestina i po zavedeni prepinace.
    fn default() -> Self {
        Lang::Cs
    }
}

impl Lang {
    /// Vsechny podporovane jazyky, v poradi pro zobrazeni v dropdownu.
    pub const ALL: [Lang; 2] = [Lang::Cs, Lang::En];

    /// Jmeno jazyka VZDY napsane v tom jazyce samotnem (ne v aktualne
    /// zvolenem UI jazyce) - napr. "Čeština", ne "Czech" - aby zustalo
    /// citelne, i kdyz uzivatel omylem prepne na jazyk, ktery neumi
    /// cist, a potrebuje se v dropdownu zorientovat zpet.
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::Cs => "Čeština",
            Lang::En => "English",
        }
    }
}

/// Katalog vsech prekladatelnych UI textu - viz komentar u modulu.
pub struct Strings {
    // -- spolecne popisky poli (formulare serveru: Home pripojeni, Novy
    // server, Upravit server, Rychle pripojeni - vsechny sdili stejnou
    // sadu poli) --
    pub field_name: &'static str,
    pub field_folder: &'static str,
    pub field_host: &'static str,
    pub field_port: &'static str,
    pub field_username: &'static str,
    pub field_password: &'static str,

    // -- spolecna tlacitka --
    pub btn_add: &'static str,
    pub btn_save: &'static str,
    pub btn_cancel: &'static str,
    pub btn_connect: &'static str,
    pub btn_create: &'static str,
    pub btn_move: &'static str,
    pub btn_delete: &'static str,
    pub btn_close: &'static str,
    pub btn_change: &'static str,
    pub btn_export: &'static str,
    pub btn_import: &'static str,
    pub btn_browse: &'static str,
    pub btn_select_all: &'static str,
    pub btn_select_none: &'static str,
    pub btn_open: &'static str,
    pub btn_edit: &'static str,
    pub btn_rename: &'static str,
    pub btn_move_to_folder: &'static str,
    pub btn_rename_folder: &'static str,
    pub btn_delete_empty_folder: &'static str,
    pub btn_new_server: &'static str,
    pub btn_new_folder: &'static str,
    pub btn_reconnect: &'static str,
    pub btn_open_release_page: &'static str,
    pub btn_export_vault: &'static str,
    pub btn_import_vault: &'static str,
    pub btn_create_vault: &'static str,

    // -- Home tab (`MainApp::render_home`) --
    pub home_heading: &'static str,
    pub home_save_checkbox: &'static str,
    pub home_save_hint: &'static str,
    pub home_guest_note: &'static str,
    pub version_label: &'static str,
    pub checking_update: &'static str,
    pub up_to_date: &'static str,

    // -- horni menu (`MainApp::top_menu`) --
    pub menu_terminal: &'static str,
    pub menu_terminal_exit: &'static str,
    pub menu_sessions: &'static str,
    pub menu_sessions_new_server: &'static str,
    pub menu_sessions_new_folder: &'static str,
    pub menu_sessions_new_quick_connect: &'static str,
    pub menu_view: &'static str,
    pub menu_tools: &'static str,
    pub menu_settings: &'static str,
    pub menu_help: &'static str,

    // -- taby (`MainApp::tab_title`) --
    pub tab_home: &'static str,
    pub tab_settings: &'static str,
    pub tab_connection_fallback: &'static str,
    pub connection_gone: &'static str,

    // -- dialog: Novy/Upravit server --
    pub dialog_new_server_title: &'static str,
    pub dialog_edit_server_title: &'static str,

    // -- dialog: Nova slozka --
    pub dialog_new_folder_title: &'static str,
    pub new_folder_path_hint: &'static str,

    // -- dialog: Prejmenovat --
    pub dialog_rename_title: &'static str,

    // -- dialog: Presunout do slozky --
    pub dialog_move_title: &'static str,
    pub move_folder_path_hint: &'static str,

    // -- dialog: Smazat --
    pub dialog_delete_title: &'static str,

    // -- dialog: Zavrit spojeni --
    pub dialog_close_connection_title: &'static str,

    // -- dialog: Zmenit heslo trezoru --
    pub dialog_change_password_title: &'static str,
    pub current_password_label: &'static str,
    pub new_password_label: &'static str,
    pub repeat_new_password_label: &'static str,
    pub current_password_wrong: &'static str,
    pub new_password_empty: &'static str,
    pub new_passwords_mismatch: &'static str,
    pub vault_password_changed: &'static str,

    // -- dialog: Nove rychle spojeni --
    pub dialog_quick_connect_title: &'static str,
    pub quick_connect_note: &'static str,

    // -- dialog: Exportovat trezor --
    pub dialog_export_title: &'static str,
    pub export_what_label: &'static str,
    pub export_search_hover: &'static str,
    pub export_empty_vault: &'static str,
    pub export_target_file_label: &'static str,
    pub export_password_label: &'static str,
    pub repeat_password_label: &'static str,
    pub export_password_note: &'static str,
    pub export_target_missing: &'static str,
    pub export_selection_empty: &'static str,
    pub export_password_empty: &'static str,
    pub passwords_mismatch: &'static str,
    pub export_save_dialog_title: &'static str,
    pub vault_file_filter_name: &'static str,

    // -- dialog: Importovat trezor --
    pub dialog_import_title: &'static str,
    pub import_source_file_label: &'static str,
    pub import_password_label: &'static str,
    pub import_replace_checkbox: &'static str,
    pub import_merge_note: &'static str,
    pub import_source_missing: &'static str,
    pub import_success: &'static str,
    pub import_open_dialog_title: &'static str,

    // -- strom serveru (`MainApp::show_tree`/`render_folder_contents`/`render_session_row`) --
    pub vault_save_failed: &'static str,

    // -- hostovsky rezim: prihlasovaci formular v levem panelu
    // (`render_guest_login`) --
    pub guest_mode_heading: &'static str,
    pub guest_mode_hint: &'static str,
    pub guest_login_prompt: &'static str,
    pub guest_create_prompt: &'static str,
    pub main_password_hint: &'static str,
    pub repeat_password_hint: &'static str,
    pub btn_login: &'static str,
    pub vault_password_warning: &'static str,
    pub main_password_empty: &'static str,
    pub vault_unlock_failed: &'static str,
    pub vault_create_failed: &'static str,

    // -- Nastaveni (`MainApp::render_settings`) --
    pub settings_heading: &'static str,
    pub settings_guest_note: &'static str,
    pub settings_vault_location: &'static str,
    pub settings_appearance_heading: &'static str,
    pub settings_theme_note: &'static str,
    pub settings_language_heading: &'static str,
    pub settings_language_label: &'static str,
    pub settings_ssh_loss_heading: &'static str,
    pub settings_auto_reconnect_checkbox: &'static str,
    pub settings_auto_reconnect_note: &'static str,

    // -- zamcena obrazovka (`TermxApp::render_lock_screen`) --
    pub lock_unlock_prompt: &'static str,
    pub lock_create_prompt: &'static str,
    pub btn_unlock: &'static str,
    pub btn_continue_without_password: &'static str,
    pub lock_guest_note: &'static str,

    // -- terminal (`terminal::TerminalSession::render`/`render_status_bar`) --
    pub connecting_label: &'static str,
    pub connection_ended: &'static str,
    pub auto_reconnect_hint: &'static str,
    pub status_host_tooltip: &'static str,
    pub status_cpu_tooltip: &'static str,
    pub status_mem_tooltip: &'static str,
    pub status_net_up_tooltip: &'static str,
    pub status_net_down_tooltip: &'static str,
    pub status_uptime_tooltip: &'static str,
    pub status_user_tooltip: &'static str,
    pub status_disk_tooltip: &'static str,
    pub day_singular: &'static str,
    pub day_few: &'static str,
    pub day_plural: &'static str,
}

pub const CS: Strings = Strings {
    field_name: "Název:",
    field_folder: "Složka:",
    field_host: "Host:",
    field_port: "Port:",
    field_username: "Uživatel:",
    field_password: "Heslo:",

    btn_add: "Přidat",
    btn_save: "Uložit",
    btn_cancel: "Zrušit",
    btn_connect: "Připojit",
    btn_create: "Vytvořit",
    btn_move: "Přesunout",
    btn_delete: "Smazat",
    btn_close: "Zavřít",
    btn_change: "Změnit",
    btn_export: "Exportovat",
    btn_import: "Importovat",
    btn_browse: "Procházet...",
    btn_select_all: "Vybrat vše",
    btn_select_none: "Nic nevybírat",
    btn_open: "Otevřít",
    btn_edit: "Upravit...",
    btn_rename: "Přejmenovat...",
    btn_move_to_folder: "Přesunout do složky...",
    btn_rename_folder: "Přejmenovat složku...",
    btn_delete_empty_folder: "Smazat prázdnou složku",
    btn_new_server: "+ Server",
    btn_new_folder: "+ Složka",
    btn_reconnect: "🔄 Připojit znovu",
    btn_open_release_page: "Otevřít stránku s vydáním",
    btn_export_vault: "Exportovat trezor...",
    btn_import_vault: "Importovat trezor...",
    btn_create_vault: "Vytvořit trezor",

    home_heading: "Připojit k novému serveru",
    home_save_checkbox: "Uložit server do trezoru",
    home_save_hint: "Když je zaškrtnuto, server se uloží do trezoru a objeví se ve stromu vlevo. \
                      Když ne, jde jen o dočasné rychlé spojení (zmizí se zavřením tabu/aplikace).",
    home_guest_note: "Hostovský režim: spojení se neukládá, jde vždy jen o dočasné rychlé spojení.",
    version_label: "verze",
    checking_update: "Kontroluji dostupnost aktualizace…",
    up_to_date: "Máte nejnovější verzi.",

    menu_terminal: "Terminál",
    menu_terminal_exit: "Ukončit",
    menu_sessions: "Servery",
    menu_sessions_new_server: "Nový server...",
    menu_sessions_new_folder: "Nová složka...",
    menu_sessions_new_quick_connect: "Nové rychlé spojení...",
    menu_view: "Zobrazení",
    menu_tools: "Nástroje",
    menu_settings: "Nastavení",
    menu_help: "Nápověda",

    tab_home: "Domů",
    tab_settings: "Nastavení",
    tab_connection_fallback: "Spojení",
    connection_gone: "Tento server už neexistuje (byl smazán nebo šlo o dočasné rychlé spojení, které skončilo se zavřením tabu).",

    dialog_new_server_title: "Nový server",
    dialog_edit_server_title: "Upravit server",

    dialog_new_folder_title: "Nová složka",
    new_folder_path_hint: "Cesta nové složky (např. Práce/Nová):",

    dialog_rename_title: "Přejmenovat",

    dialog_move_title: "Přesunout do složky",
    move_folder_path_hint: "Cesta složky (např. Práce/PBX), prázdné = kořenová úroveň:",

    dialog_delete_title: "Smazat",

    dialog_close_connection_title: "Zavřít spojení",

    dialog_change_password_title: "Změnit heslo trezoru",
    current_password_label: "Současné heslo:",
    new_password_label: "Nové heslo:",
    repeat_new_password_label: "Zopakujte nové heslo:",
    current_password_wrong: "Současné heslo je nesprávné.",
    new_password_empty: "Nové heslo nesmí být prázdné.",
    new_passwords_mismatch: "Zadaná nová hesla se neshodují.",
    vault_password_changed: "Heslo trezoru bylo změněno.",

    dialog_quick_connect_title: "Nové rychlé spojení",
    quick_connect_note: "Toto spojení se nikam neukládá - platí jen do zavření tabu/aplikace.",

    dialog_export_title: "Exportovat trezor",
    export_what_label: "Co exportovat:",
    export_search_hover: "Hledat podle názvu serveru, hostu nebo složky",
    export_empty_vault: "Trezor je prázdný - není co exportovat.",
    export_target_file_label: "Cílový soubor:",
    export_password_label: "Heslo exportu:",
    repeat_password_label: "Zopakujte heslo:",
    export_password_note: "Heslo exportu může být jiné než hlavní heslo trezoru - hodí se \
                            např. při předání serverů kolegovi.",
    export_target_missing: "Zadejte cílový soubor.",
    export_selection_empty: "Vyberte alespoň jeden server nebo složku k exportu.",
    export_password_empty: "Heslo exportu nesmí být prázdné.",
    passwords_mismatch: "Zadaná hesla se neshodují.",
    export_save_dialog_title: "Exportovat trezor jako...",
    vault_file_filter_name: "Term-IX trezor",

    dialog_import_title: "Importovat trezor",
    import_source_file_label: "Zdrojový soubor:",
    import_password_label: "Heslo souboru:",
    import_replace_checkbox: "Nahradit aktuální trezor (místo sloučení)",
    import_merge_note: "Sloučení přidá importované servery a složky k těm stávajícím. \
                         Nahrazení aktuální trezor kompletně přepíše obsahem importu.",
    import_source_missing: "Zadejte zdrojový soubor.",
    import_success: "Trezor byl úspěšně importován.",
    import_open_dialog_title: "Importovat trezor...",

    vault_save_failed: "Uložení trezoru selhalo",

    guest_mode_heading: "Hostovský režim",
    guest_mode_hint: "Uložené servery nejsou vidět.",
    guest_login_prompt: "Přihlásit se k trezoru:",
    guest_create_prompt: "Trezor ještě neexistuje - nastavte heslo:",
    main_password_hint: "Hlavní heslo",
    repeat_password_hint: "Zopakujte heslo",
    btn_login: "Přihlásit",
    vault_password_warning: "Pozor: při zapomenutí tohoto hesla se k uloženým údajům už nikdo nedostane.",
    main_password_empty: "Hlavní heslo nesmí být prázdné.",
    vault_unlock_failed: "Nepodařilo se odemknout trezor",
    vault_create_failed: "Nepodařilo se vytvořit trezor",

    settings_heading: "Nastavení",
    settings_guest_note: "Jste přihlášeni v hostovském režimu (bez hlavního hesla) - žádný trezor \
                           se nečte ani nezapisuje. Pro přístup k uloženým serverům aplikaci restartujte \
                           a zadejte hlavní heslo.",
    settings_vault_location: "Umístění trezoru:",
    settings_appearance_heading: "Vzhled",
    settings_theme_note: "V této verzi je k dispozici jen 'terminálové' tmavé téma. \
                           Modernější téma přibude jako další volba zde.",
    settings_language_heading: "Jazyk aplikace",
    settings_language_label: "Jazyk:",
    settings_ssh_loss_heading: "Ztráta SSH spojení",
    settings_auto_reconnect_checkbox: "Automaticky se pokoušet obnovit ztracené spojení",
    settings_auto_reconnect_note: "Když je vypnuto (výchozí), spojení po odpojení zůstane přerušené a příslušný \
                                    tab se jen zbarví, aby bylo na první pohled vidět, že je mrtvé - obnovit ho pak \
                                    jde ručně tlačítkem přímo v tabu terminálu. Když je zapnuto, aplikace se navíc \
                                    sama periodicky pokouší spojení obnovit.",

    lock_unlock_prompt: "Zadejte hlavní heslo trezoru:",
    lock_create_prompt: "Trezor ještě neexistuje – nastavte hlavní heslo:",
    btn_unlock: "Odemknout",
    btn_continue_without_password: "Pokračovat bez hesla",
    lock_guest_note: "Hostovský režim: uložené servery nejsou vidět a nová se neukládají - \
                       jen rychlé spojení bez uložení.",

    connecting_label: "Připojuji…",
    connection_ended: "Spojení bylo ukončeno.",
    auto_reconnect_hint: "(automaticky se zkouší obnovit)",
    status_host_tooltip: "Adresa (hostname/IP) fyzického serveru, ke kterému je toto spojení připojeno.",
    status_cpu_tooltip: "Vytížení CPU serveru (odhad z 1minutového průměru zátěže vydělený počtem jader).",
    status_mem_tooltip: "Využitá a celková operační paměť (RAM) serveru.",
    status_net_up_tooltip: "Aktuální rychlost odesílání dat ze serveru (upload).",
    status_net_down_tooltip: "Aktuální rychlost přijímání dat na serveru (download).",
    status_uptime_tooltip: "Jak dlouho server běží od posledního restartu.",
    status_user_tooltip: "Přihlášený uživatel a počet jeho aktivních přihlášených relací na serveru.",
    status_disk_tooltip: "Zaplnění kořenového disku (/) na serveru.",
    day_singular: "den",
    day_few: "dny",
    day_plural: "dní",
};

pub const EN: Strings = Strings {
    field_name: "Name:",
    field_folder: "Folder:",
    field_host: "Host:",
    field_port: "Port:",
    field_username: "Username:",
    field_password: "Password:",

    btn_add: "Add",
    btn_save: "Save",
    btn_cancel: "Cancel",
    btn_connect: "Connect",
    btn_create: "Create",
    btn_move: "Move",
    btn_delete: "Delete",
    btn_close: "Close",
    btn_change: "Change",
    btn_export: "Export",
    btn_import: "Import",
    btn_browse: "Browse...",
    btn_select_all: "Select all",
    btn_select_none: "Select none",
    btn_open: "Open",
    btn_edit: "Edit...",
    btn_rename: "Rename...",
    btn_move_to_folder: "Move to folder...",
    btn_rename_folder: "Rename folder...",
    btn_delete_empty_folder: "Delete empty folder",
    btn_new_server: "+ Server",
    btn_new_folder: "+ Folder",
    btn_reconnect: "🔄 Reconnect",
    btn_open_release_page: "Open release page",
    btn_export_vault: "Export vault...",
    btn_import_vault: "Import vault...",
    btn_create_vault: "Create vault",

    home_heading: "Connect to a new server",
    home_save_checkbox: "Save server to the vault",
    home_save_hint: "When checked, the server is saved to the vault and appears in the tree on the left. \
                      When not, it's just a temporary quick connection (disappears when the tab/app is closed).",
    home_guest_note: "Guest mode: the connection isn't saved, it's always just a temporary quick connection.",
    version_label: "version",
    checking_update: "Checking for updates…",
    up_to_date: "You have the latest version.",

    menu_terminal: "Terminal",
    menu_terminal_exit: "Exit",
    menu_sessions: "Sessions",
    menu_sessions_new_server: "New server...",
    menu_sessions_new_folder: "New folder...",
    menu_sessions_new_quick_connect: "New quick connection...",
    menu_view: "View",
    menu_tools: "Tools",
    menu_settings: "Settings",
    menu_help: "Help",

    tab_home: "Home",
    tab_settings: "Settings",
    tab_connection_fallback: "Connection",
    connection_gone: "This server no longer exists (it was deleted, or it was a temporary quick connection that ended when its tab was closed).",

    dialog_new_server_title: "New server",
    dialog_edit_server_title: "Edit server",

    dialog_new_folder_title: "New folder",
    new_folder_path_hint: "New folder path (e.g. Work/New):",

    dialog_rename_title: "Rename",

    dialog_move_title: "Move to folder",
    move_folder_path_hint: "Folder path (e.g. Work/PBX), empty = root level:",

    dialog_delete_title: "Delete",

    dialog_close_connection_title: "Close connection",

    dialog_change_password_title: "Change vault password",
    current_password_label: "Current password:",
    new_password_label: "New password:",
    repeat_new_password_label: "Repeat new password:",
    current_password_wrong: "Current password is incorrect.",
    new_password_empty: "New password must not be empty.",
    new_passwords_mismatch: "The new passwords entered don't match.",
    vault_password_changed: "Vault password has been changed.",

    dialog_quick_connect_title: "New quick connection",
    quick_connect_note: "This connection isn't saved anywhere - it only lasts until the tab/app is closed.",

    dialog_export_title: "Export vault",
    export_what_label: "What to export:",
    export_search_hover: "Search by server name, host, or folder",
    export_empty_vault: "The vault is empty - there's nothing to export.",
    export_target_file_label: "Destination file:",
    export_password_label: "Export password:",
    repeat_password_label: "Repeat password:",
    export_password_note: "The export password can differ from the vault's main password - handy \
                            e.g. when handing servers over to a colleague.",
    export_target_missing: "Enter a destination file.",
    export_selection_empty: "Select at least one server or folder to export.",
    export_password_empty: "Export password must not be empty.",
    passwords_mismatch: "The passwords entered don't match.",
    export_save_dialog_title: "Export vault as...",
    vault_file_filter_name: "Term-IX vault",

    dialog_import_title: "Import vault",
    import_source_file_label: "Source file:",
    import_password_label: "File password:",
    import_replace_checkbox: "Replace the current vault (instead of merging)",
    import_merge_note: "Merging adds the imported servers and folders to the existing ones. \
                         Replacing completely overwrites the current vault with the import's content.",
    import_source_missing: "Enter a source file.",
    import_success: "The vault was imported successfully.",
    import_open_dialog_title: "Import vault...",

    vault_save_failed: "Saving the vault failed",

    guest_mode_heading: "Guest mode",
    guest_mode_hint: "Saved servers aren't visible.",
    guest_login_prompt: "Log in to the vault:",
    guest_create_prompt: "The vault doesn't exist yet - set a password:",
    main_password_hint: "Main password",
    repeat_password_hint: "Repeat password",
    btn_login: "Log in",
    vault_password_warning: "Warning: if you forget this password, nobody will ever be able to access the stored data again.",
    main_password_empty: "Main password must not be empty.",
    vault_unlock_failed: "Failed to unlock the vault",
    vault_create_failed: "Failed to create the vault",

    settings_heading: "Settings",
    settings_guest_note: "You're logged in in guest mode (no main password) - no vault is being \
                           read from or written to. To access saved servers, restart the app \
                           and enter the main password.",
    settings_vault_location: "Vault location:",
    settings_appearance_heading: "Appearance",
    settings_theme_note: "This version only offers the 'terminal-style' dark theme. \
                           A more modern theme will be added as another option here.",
    settings_language_heading: "Application language",
    settings_language_label: "Language:",
    settings_ssh_loss_heading: "SSH connection loss",
    settings_auto_reconnect_checkbox: "Automatically try to restore a lost connection",
    settings_auto_reconnect_note: "When off (default), a disconnected connection stays interrupted and its \
                                    tab just changes color so it's clear at a glance it's dead - it can then \
                                    be restored manually with the button right in the terminal tab. When on, \
                                    the app additionally tries to restore the connection by itself periodically.",

    lock_unlock_prompt: "Enter the vault's main password:",
    lock_create_prompt: "The vault doesn't exist yet – set a main password:",
    btn_unlock: "Unlock",
    btn_continue_without_password: "Continue without a password",
    lock_guest_note: "Guest mode: saved servers aren't visible and new ones aren't saved - \
                       quick connections only, nothing is stored.",

    connecting_label: "Connecting…",
    connection_ended: "The connection has ended.",
    auto_reconnect_hint: "(automatically trying to restore)",
    status_host_tooltip: "Address (hostname/IP) of the physical server this connection is connected to.",
    status_cpu_tooltip: "Server CPU load (estimated from the 1-minute load average divided by the core count).",
    status_mem_tooltip: "Used and total physical memory (RAM) on the server.",
    status_net_up_tooltip: "Current data upload speed from the server.",
    status_net_down_tooltip: "Current data download speed to the server.",
    status_uptime_tooltip: "How long the server has been running since its last restart.",
    status_user_tooltip: "Logged-in user and their number of active logged-in sessions on the server.",
    status_disk_tooltip: "Root disk (/) usage on the server.",
    day_singular: "day",
    day_few: "days",
    day_plural: "days",
};

/// Vrati katalog textu pro dany jazyk.
pub fn t(lang: Lang) -> &'static Strings {
    match lang {
        Lang::Cs => &CS,
        Lang::En => &EN,
    }
}

// -- pomocne funkce pro texty s dosazovanym obsahem (verze, jmeno
// souboru, chybova hlaska od `anyhow`/`termx-vault` apod.) - viz
// komentar u modulu, proc NE prosty stringly-keyovany format!() na
// zavolani. --

pub fn update_check_failed(lang: Lang, err: &str) -> String {
    match lang {
        Lang::Cs => format!("Kontrolu aktualizace se nepodařilo provést ({err})."),
        Lang::En => format!("Failed to check for updates ({err})."),
    }
}

pub fn update_available(lang: Lang, version: &str) -> String {
    match lang {
        Lang::Cs => format!("Dostupná je nová verze {version}."),
        Lang::En => format!("A new version {version} is available."),
    }
}

pub fn protocol_not_supported(lang: Lang, protocol: impl std::fmt::Display) -> String {
    match lang {
        Lang::Cs => format!("Protokol {protocol} zatím nemá vestavěný terminál - podporováno je prozatím jen SSH."),
        Lang::En => format!("Protocol {protocol} doesn't have a built-in terminal yet - only SSH is supported for now."),
    }
}

pub fn confirm_delete_server(lang: Lang, name: &str) -> String {
    match lang {
        Lang::Cs => format!("Opravdu smazat server „{name}“?"),
        Lang::En => format!("Are you sure you want to delete the server “{name}”?"),
    }
}

pub fn confirm_delete_folder(lang: Lang, path: &str) -> String {
    match lang {
        Lang::Cs => format!("Opravdu smazat prázdnou složku „{path}“?"),
        Lang::En => format!("Are you sure you want to delete the empty folder “{path}”?"),
    }
}

pub fn confirm_close_connection(lang: Lang, title: &str) -> String {
    match lang {
        Lang::Cs => format!("Spojení „{title}“ je právě aktivní. Opravdu chcete tab zavřít a spojení ukončit?"),
        Lang::En => format!("The connection “{title}” is currently active. Are you sure you want to close the tab and end the connection?"),
    }
}

pub fn export_saved(lang: Lang, path: &str, servers: usize, folders: usize) -> String {
    match lang {
        Lang::Cs => format!("Trezor exportován do {path} ({servers} serverů, {folders} složek)"),
        Lang::En => format!("Vault exported to {path} ({servers} servers, {folders} folders)"),
    }
}

pub fn export_failed(lang: Lang, err: impl std::fmt::Display) -> String {
    match lang {
        Lang::Cs => format!("Export selhal: {err}"),
        Lang::En => format!("Export failed: {err}"),
    }
}

pub fn import_failed(lang: Lang, err: impl std::fmt::Display) -> String {
    match lang {
        Lang::Cs => format!("Import selhal: {err}"),
        Lang::En => format!("Import failed: {err}"),
    }
}

pub fn connection_failed(lang: Lang, err: &str) -> String {
    match lang {
        Lang::Cs => format!("Spojení skončilo chybou: {err}"),
        Lang::En => format!("The connection ended with an error: {err}"),
    }
}

/// Pocet dnu bezu serveru, se spravnou ceskou pluralizaci (1 den / 2-4
/// dny / 5+ dní) - anglictina ma jen jednotne/mnozne cislo.
pub fn uptime_days(lang: Lang, days: u64) -> String {
    let s = t(lang);
    let word = match lang {
        Lang::Cs => match days {
            1 => s.day_singular,
            2..=4 => s.day_few,
            _ => s.day_plural,
        },
        Lang::En => {
            if days == 1 {
                s.day_singular
            } else {
                s.day_plural
            }
        }
    };
    format!("{days} {word}")
}
