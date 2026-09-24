# ArchGen: posúdenie a odporúčané vylepšenia

Dátum: 2026-09-24. Cieľ: aby bol ArchGen použiteľný na bežnú dokumentáciu a architektonické návrhy,
napojený na Git a schopný tvoriť release notes podľa šablóny.

„Learn" v zadaní chápem ako **Microsoft Learn** – najmä Azure Architecture Center a Azure
Well-Architected Framework (ADR, design review podľa piatich pilierov, šablóny návrhových
dokumentov, docs-as-code s metadátami a `toc.yml`). Ak bolo myslené niečo iné, oprav ma –
odporúčania v časti B sa podľa toho upravia.

## A. Čo je hotové v tomto prírastku

| Oblasť | Zmena |
|---|---|
| Git | Stav priečinka projektu, Commit… (iba priečinok projektu, iba uložený obsah), Push do upstreamu po potvrdení |
| Release notes | Z rozsahu commitov ľubovoľného repozitára podľa šablóny; 3 vstavané šablóny, vlastné šablóny v projekte, vloženie ako dokument/sekcia, súbor |
| Šablóny dokumentov | + Solution Design (štruktúra podľa Azure Architecture Center / piatich pilierov), ADR (MADR), Well-Architected Review, Release Notes; návod ku každej sekcii |
| Strata dát | Výber šablóny už nemaže napísaný obsah |
| Exporty | Markdown s front matter (kompatibilné s DocFX/Learn, MkDocs, Hugo, wiki), HTML so skutočne vykreslenými PlantUML diagramami a obsahom; Visio/EA už nepredstierajú úspech |

Podrobnosti: [IMPLEMENTATION-PROGRESS.md](IMPLEMENTATION-PROGRESS.md), návod: [GIT-A-RELEASE-NOTES.md](GIT-A-RELEASE-NOTES.md).

## B. Odporúčané ďalšie kroky podľa priority

### P1 – bez toho architekt narazí hneď

1. **C4 diagramy v lokálnom rendereri.** C4-PlantUML je štandard pre architektúru, ale renderer
   dnes odmieta všetky `!include`. PlantUML má C4 knižnicu priamo v `plantuml.jar`
   (`!include <C4/C4_Container>`), takže stačí povoliť whitelist `!include <C4/…>` (a iné stdlib
   `<…>`), stále bez `!includeurl` a súborových include. Veľký prínos, malé riziko.
2. **Lokálny Mermaid.** GitHub, GitLab aj Azure DevOps Mermaid vykresľujú priamo; v ArchGen sa
   zobrazí iba zdroj. Balík `mermaid` vo webview so `securityLevel: "strict"` a sanitizovaným SVG
   (rovnako ako `local-svg.ts`) to vyrieši bez siete.
3. **Docs/Diagram cez rovnakého AI providera ako Rework.** Nastavenia AI dnes platia iba pre
   Rework; Docs/Diagram idú cez Python a Ollamu. Jednotné `ai::complete` so schémou sekcií
   odstráni dve konfigurácie a umožní štruktúrované výstupy aj pre generovanie.
4. **Export celého projektu ako dokumentačného webu.** Priečinok s `index.md`, stránkou na
   dokument, SVG diagramami vedľa Markdownu a `toc.yml` (DocFX / Microsoft Learn) alebo
   `mkdocs.yml`. Front matter je už pripravený.

### P2 – kvalita architektonickej práce

5. **Register ADR.** Číslovanie `ADR-0001`, stav (proposed/accepted/superseded), odkaz
   „superseded by", prehľad všetkých rozhodnutí projektu; odkaz z arc42 kapitoly 9.
6. **Metadáta dokumentu** v `document.json`: vlastník, stav (draft/review/approved), verzia,
   dátum revízie – zobrazené v UI a v exportoch (ako metadáta článkov na Microsoft Learn).
7. **Odkazy medzi dokumentmi a spoločný glosár.** Stabilné ID sekcií už existujú; chýba syntax
   odkazu (`[[doc#sekcia]]`) a jeho preklad pri exporte.
8. **Review cez Git** (docs-as-code): vetva pre návrh, commit, push a odkaz na vytvorenie PR
   (GitHub/Azure DevOps). Commit a push sú hotové; chýba vetva a PR odkaz.
9. **Design review checklist** podľa Well-Architected (úlohy s `- [ ]` pre každý pilier) a jeho
   vyhodnotenie – prirodzene nadväzuje na šablónu Well-Architected Review.
10. **Release notes ďalej:** voliteľné zhrnutie cez AI priamo v dialógu (dnes cez Rework po
    vložení), názvy a štítky PR z GitHub/Azure DevOps namiesto správ commitov, vytvorenie
    anotovaného tagu po schválení poznámok.

### P3 – prevádzka a bezpečnosť

11. **CSP** v `tauri.conf.json` (dnes `null`); aplikácia už nič vzdialené nepotrebuje okrem
    voliteľného verejného PlantUML servera.
12. **Single-instance ochrana** – druhá inštancia dnes prerušuje úlohy prvej.
13. **Visio/EA**: buď skutočná implementácia (EA automation, VSDX), alebo export do formátu,
    ktorý tieto nástroje importujú (XMI pre EA, draw.io/VSDX pre Visio) bez COM.
14. **Mimo Windows**: kód sa po tomto prírastku preloží aj na Linuxe; DPAPI a Job Objects sú
    jediné Windows-only časti. Keychain/libsecret a process groups by otvorili macOS/Linux.
15. **Automatické CI** (GitHub Actions na Windows): `npm test`, `npm run check`, e2e, `cargo test`.
