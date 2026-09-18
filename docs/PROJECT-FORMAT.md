# Formát projektu na disku

Projekt je obyčajný priečinok s textovými súbormi, určený do Git repozitára. Dá sa
čítať, porovnávať a upravovať aj bez ArchGen — v IDE, na GitLabe alebo AI agentom.

```text
moj-projekt/
  archgen.json                          id, názov, poradie dokumentov
  documents/
    architektura/
      document.json                     id, názov, šablóna, jazyk, poradie sekcií a diagramov
      1-introduction-and-goals.md       "# Nadpis" + Markdown obsah sekcie
      5-building-block-view.md
      5-building-block-view.class.puml  zdroj diagramu (PlantUML → .puml, Mermaid → .mmd)
```

## Pravidlá

- **Na disk nejdú revízie ani aktívny tab.** Sú to údaje relácie; v Gite by menili
  súbory pri každom uložení a spôsobovali konflikty. Uloženie prepíše iba súbory,
  ktorých obsah sa zmenil. Revízie posledného známeho obsahu drží lokálny register
  (`runtime.sqlite3` v app-data), takže hotové AI návrhy prežijú reštart.
- **Názvy súborov** vznikajú z názvov (ASCII, malé písmená, bez diakritiky, najviac
  60 znakov, kolízie dostanú `-2`, `-3`). Premenovanie sekcie premenuje súbor; Git to
  rozpozná ako rename.
- **Nadpis sekcie** je prvý riadok `# Nadpis` v `.md`. Ak ho riadok nevie presne
  niesť (zalomenie, okrajové medzery, prázdny), presná hodnota je v `document.json`
  ako `title`.
- **ID sú v manifestoch.** Pri ručne pridanej sekcii alebo diagrame ich netreba
  písať — ArchGen ich pri načítaní doplní a pri najbližšom uložení zapíše:

  ```json
  { "file": "nova-sekcia.md", "diagrams": [{ "type": "sequence", "format": "plantuml", "file": "tok.puml" }] }
  ```
- **Konce riadkov:** vnútri ArchGen je vždy LF. CRLF po `git checkout` sa nepovažuje
  za zmenu a súbor sa kvôli nemu neprepíše. Odporúčaný `.gitattributes`: `* text=auto eol=lf`.
- **Ochrana cudzích súborov:** ArchGen maže iba súbory, ktoré menoval predchádzajúci
  manifest, a nikdy neprepíše súbor, ktorý k projektu nepatril. Save as odmietne
  priečinok, v ktorom už je `archgen.json` alebo `documents/`.
- **Zmena mimo ArchGen** (git pull, checkout, iný editor) počas otvoreného projektu:
  uloženie sa odmietne, rozpracovaný obsah zostane v aplikácii a dá sa uložiť cez
  Save as. Kontroluje sa odtlačok obsahu, nie čas súboru.
- Manifesty z cudzieho repozitára sú nedôveryhodný vstup: názvy súborov musia byť
  jedna bežná zložka cesty (žiadne `..`, `/`, `\`, `:`), celkový obsah najviac 32 MiB.

## História

Git history v aplikácii číta `git log` pre priečinok projektu a obsah commitu cez
`git cat-file`, bez checkoutu a bez zmeny repozitára. Restore vytvorí novú pracovnú
revíziu; do Gitu sa dostane až tvojím uložením a commitom. ArchGen sám necommituje.
Neuložené a necommitnuté uloženia v histórii nie sú.

## Staré `.archgen` súbory

Pôvodný jednosúborový SQLite formát sa už nezapisuje. **Import .archgen** načíta
jeho posledný checkpoint (súbor nemení) ako neuložený projekt; Save project mu
vyberie priečinok.
