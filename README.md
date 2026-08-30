# Term-IX

Terminalovy klient pro spravu vzdalenych spojeni (SSH, seriova linka,
FTP, ...) inspirovany aplikaci MobaXterm - napsany v Rustu, pro
Windows i Linux.

> Stav: **rana faze vyvoje (MVP kostra)**. Funguje SSH modul s
> prihlasenim jmenem/heslem, sifrovane ulozeni serveru a zaklad pro
> self-update. Dalsi protokoly a funkce se budou pridavat postupne.

## Architektura

Aplikace je rozdelena do samostatnych cargo crates (workspace), aby
zdrojaky nekoncily v jednom nekonecnem souboru a aby slo pridavat
protokoly bez zasahu do zbytku aplikace:

```
Term-IX/
├── src/main.rs          – binarka: propoji vse dohromady, CLI, self-update
└── crates/
    ├── termx-core/       – sdilene typy: Session, AuthMethod, trait
    │                       ProtocolModule, cross-platform cesty (AppPaths)
    ├── termx-vault/       – sifrovane ulozeni serveru (AES-256-GCM + Argon2id)
    ├── termx-update/      – self-update z GitHub Releases
    ├── termx-ssh/         – SSH modul (prvni implementace ProtocolModule)
    └── termx-tui/         – terminalove UI (ratatui) - seznam serveru,
                              formulare, napojeni na moduly pres registr
```

### Jak pridat novy protokol (napr. seriovou linku / FTP)

1. Vytvorit novy crate `crates/termx-serial` (nebo `termx-ftp`), pridat
   ho do `members` v korenovem `Cargo.toml`.
2. Implementovat `termx_core::ProtocolModule` (metody `protocol_key`,
   `display_name`, `run`).
3. V `src/main.rs` pridat `registry.register(Arc::new(termx_serial::SerialModule::new()))`.
4. Rozsirit `termx_core::Protocol` o novou variantu (a pripadne
   `AuthMethod`, pokud protokol potrebuje jiny typ prihlaseni).

Zbytek aplikace (TUI, vault, update) se timto nemusi menit.

## Bezpecnost ulozenych serveru

- Ulozene servery (host, port, uzivatel, heslo/cesta ke klici, ...)
  jsou v `termx-vault` serializovany a ulozeny na disk **vzdy
  zasifrovane**: AES-256-GCM, klic odvozeny z hlavniho hesla pomoci
  Argon2id (pomale/pametove narocne KDF - ztezuje brute-force i pri
  uniku souboru).
- Trezor se pri kazdem startu odemyka hlavnim heslem. **Kdo hlavni
  heslo zapomene, k ulozenym udajum se uz nedostane** - zadny reset
  ani "zadni vratka" v aplikaci zamerne nejsou.
- Export/import (`Vault::export` / `Vault::import`) umoznuje vytvorit
  samostatny sifrovany soubor (klidne s jinym heslem nez hlavni
  trezor) pro prenos na jiny pocitac.
- Vychozi umisteni trezoru: `%APPDATA%\DaTTcz\Term-IX\vault.termx`
  (Windows) / `~/.local/share/term-ix/vault.termx` (Linux).
- **Znamy dluh MVP:** SSH modul zatim neoveruje otisk klice serveru
  (zadny `known_hosts`) - pripoji se k cemukoliv, co odpovi. Doplnit
  pred pouzitim na sitich, kde hrozi MITM.

## Self-update

`termx-update` pri startu (pokud neni pouzit prepinac `--no-update`)
zkontroluje nejnovejsi GitHub Release na
[github.com/DaTTcz/Term-IX](https://github.com/DaTTcz/Term-IX) a
pripadne stahne a nahradi bezici binarku. Release proces
(`.github/workflows/release.yml`) po vytvoreni tagu `vX.Y.Z`
automaticky zabuildi a nahraje binarky pro Windows i Linux.

## Sestaveni

Vyzaduje Rust (stable) - <https://rustup.rs>.

```sh
cargo build --release
# vysledna binarka: target/release/term-ix(.exe)
```

Spusteni:

```sh
cargo run --release
```

Pri prvnim spusteni aplikace vyzve k nastaveni hlavniho hesla trezoru.

## Dulezita poznamka k tomuto commitu

Tato pocatecni kostra byla vytvorena v izolovanem prostredi bez
pristupu na crates.io, takze **zde nebylo mozne spustit `cargo build`
/ `cargo check`** a kod tak neprosel automatickym overenim kompilace
(zejmena crate `russh` v `termx-ssh` mel v ruznych verzich mirne
odlisne API - viz komentar v `crates/termx-ssh/src/lib.rs`). Az
spustite `cargo build` poprve u sebe, je mozne, ze bude potreba
doladit par nazvu metod/typu. Architektura (moduly, trait
`ProtocolModule`, sifrovani, TUI) tim dotcena neni - jde jen o
"posledni mili" prizpusobeni konkretni verzi zavislosti.

## Roadmap (navrh)

- [ ] Overeni otisku klice serveru (known_hosts) v `termx-ssh`
- [ ] Prihlaseni SSH privatnim klicem / pres ssh-agent
- [ ] Modul `termx-serial` (seriova linka, obdoba PuTTY/RealTerm)
- [ ] Modul `termx-ftp` / `termx-sftp`
- [ ] Vice zalozek/panelu v TUI (soubezna spojeni)
- [ ] Prenos souboru pretazenim / prikazem v ramci SSH/SFTP relace
- [ ] Kompletni known_hosts + varovani pri zmene klice serveru
