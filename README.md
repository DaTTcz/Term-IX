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
    ├── termx-tui/         – terminalove UI (ratatui) - seznam serveru,
    │                        formulare, napojeni na moduly pres registr
    └── termx-splash/      – uvodni splash okno s logem (mimo terminal)
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

## Loga a ikony

Zdrojova grafika je v `assets/`:

- `assets/term-ix_logo.png` – plne logo + napis "TERM-IX" (pouzito jako
  splash obrazek).
- `assets/term-ix_ico.png` – samotna znacka (hexagon + X), zdroj pro
  vsechny generovane ikony.
- `assets/icons/term-ix.ico` – multi-rozlisenova Windows ikona
  (16/32/48/64/128/256 px), vygenerovana z `term-ix_ico.png` pres
  ImageMagick (`convert term-ix_ico.png -define icon:auto-resize=... term-ix.ico`).
  `build.rs` ji pri buildu na Windows zabuduje do `term-ix.exe`
  (crate `winres`) - takze exe ma vlastni ikonu v prohlizeci/na hlavnim panelu.
- `assets/icons/hicolor/<velikost>x<velikost>/apps/term-ix.png` –
  stejna znacka v standardni freedesktop.org velikostni rade
  (16 az 512 px) pro instalaci na Linuxu, viz `packaging/term-ix.desktop`.
- `assets/fonts/DejaVuSansMono-Bold.ttf` – monospace font pro "terminalovy"
  vypis verze/autora na splash obrazovce (permisivni Bitstream Vera
  licence, viz `assets/fonts/LICENSE-DejaVu.txt`).

Chcete-li obrazky prehenerovat po zmene loga, staci znovu spustit
prikazy z tohoto oddilu (napr. `convert term-ix_ico.png -resize 128x128 ...`)
pro kazdou pozadovanou velikost.

### Splash obrazovka

Pri startu (pokud neni pouzity prepinac `--no-splash`) se zobrazi
samostatne bezramecke okno s logem. Pod nim se "terminalovym" zpusobem
- pismenko po pismenku, monospace pismem, s kurzorem - vypise
`Term-IX vX.Y.Z` a jmeno autora. Samotne psani (~30 ms/znak) tak
prirozene vytvori kratkou startovaci pauzu bez pocitu, ze se ceka
naprazdno: kurzor je behem psani plny, mezi radky a po dopsani obou
radku par-krat blikne a okno se pak samo zavre (nebo hned po
stisku klavesy/kliknuti). Implementace je v `termx-splash` (`minifb`
pro okno, `fontdue` pro rasterizaci pisma). Teprve po zavreni splash
okna aplikace prevezme terminal a spusti hlavni TUI.

Pokud se graficke okno nepodari otevrit (napr. beh pres SSH bez
X11/Wayland, headless server), splash se tise preskoci a aplikace
pokracuje rovnou do terminaloveho rozhrani - nikdy nesmi zablokovat
spusteni.

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

Prepinace: `--no-splash` preskoci uvodni okno s logem, `--no-update`
preskoci kontrolu aktualizaci.

## Dulezita poznamka k tomuto commitu

Tato pocatecni kostra byla vytvorena v izolovanem prostredi bez
pristupu na crates.io, takze **zde nebylo mozne spustit `cargo build`
/ `cargo check`** a kod tak neprosel automatickym overenim kompilace
(zejmena crate `russh` v `termx-ssh` a `minifb`/`fontdue` v
`termx-splash` mely v ruznych verzich mirne odlisne API - viz komentare
v prislusnych souborech). Az
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
