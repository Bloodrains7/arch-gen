# Diagramy: PlantUML a Mermaid

Sekcia dokumentu môže obsahovať diagram vo formáte `plantuml` alebo `mermaid`. Oba sa dajú
vykresliť **lokálne, offline a bez odoslania obsahu diagramu kamkoľvek** — líši sa len to,
kedy sa to deje.

## PlantUML: vykreslenie na vyžiadanie

Kým diagram nie je vykreslený, zobrazuje sa jeho zdroj. Náhľad (`PlantUMLPreview.svelte`)
ponúka dve tlačidlá:

- **Render locally (no upload)** — pošle obsah diagramu Rust príkazu `render_local_diagram`,
  ktorý ho vykreslí privátnym PlantUML/JRE bez siete (`src-tauri/src/renderer.rs`). Výsledné
  SVG prejde cez `local-svg.ts` (`localSvgUrl()`) a zobrazí sa ako `<img>` s `data:` URL —
  nikdy nie ako živé SVG v DOMe aplikácie.
- **Render using public PlantUML server (sends diagram content)** — voliteľná alternatíva:
  pošle zakódovaný obsah diagramu na `www.plantuml.com`. Spustí sa **iba explicitným
  kliknutím**; po zmene obsahu treba tlačidlo stlačiť znova (žiadny automatický fallback).

Limit lokálneho rendereru je 256 KiB na diagram; ďalšie obmedzenia (žiadne `!include`,
žiadny preprocessing) sú popísané v `docs/IMPLEMENTATION-PROGRESS.md`.

## Mermaid: automatické vykreslenie

Mermaid diagram (napr. `flowchart`, `sequenceDiagram`, `classDiagram`) sa vykresľuje
**automaticky**, bez tlačidla — pretože na rozdiel od verejného PlantUML servera tu nič
neopúšťa počítač: balík [`mermaid`](https://www.npmjs.com/package/mermaid) beží celý vo
webview a nič nesťahuje (žiadne fonty, žiadne ikonové balíčky, žiadna sieť).

- **Kedy sa vykresľuje**: pri každej zmene obsahu diagramu, s odstupom 400 ms (debounce),
  aby písanie nespúšťalo vykresľovanie na každý úder klávesu.
- **Ako**: `src/lib/mermaid.ts` (`renderMermaidSvg()`) načíta `mermaid` dynamickým
  `import()` — nie je súčasťou hlavného balíka aplikácie, sťahuje sa (z vlastných assetov
  aplikácie, nikdy zo siete) až pri prvom mermaid diagrame. Inicializuje sa raz, so
  `startOnLoad: false`, `securityLevel: "strict"` a `htmlLabels: false` — popisky sú vždy
  SVG `<text>`, nikdy HTML v `<foreignObject>`, lebo `local-svg.ts` `<foreignObject>`
  zakazuje a HTML popisok by tak jednoducho zmizol.
- **Sanitizácia**: výsledné SVG prechádza presne tým istým `localSvgUrl()` ako PlantUML —
  žiadny `<script>`, `<a>`, `<use>`, `<image>`, `href`, `style`. Keďže Mermaid farby/čiary
  rieši takmer výhradne cez vložený `<style>` blok (CSS triedy) a ten `local-svg.ts`
  zakazuje rovnako ako `<foreignObject>` (obsah CSS pravidla sa na rozdiel od atribútu
  nedá skenovať na `url(...)`, takže jeho povolenie by tú istú dieru znova otvorilo),
  `mermaid.ts` pred vrátením SVG sám prepočíta štýl každého elementu
  (`getComputedStyle` v skrytom, dočasnom DOM uzle) a zapíše ho ako obyčajné SVG atribúty
  (`fill`, `stroke`, `font-family`, …). Vďaka tomu diagram po sanitizácii zostáva čitateľný
  — text aj tvary majú farbu. Odkazy `url(#id)` do toho istého SVG (šípky na hranách cez
  `marker-end`, gradienty, tieň) sanitizácia ponechá, lebo nič nesťahujú; každé iné
  `url(...)` (aj zapísané CSS escape sekvenciou) odstráni — pri PlantUML rovnako.
- **Zdroj, ktorý by mohol niečo stiahnuť, sa nevykreslí**: Mermaid počas vykresľovania
  vkladá SVG so `<style>` blokom do živej stránky, a CSS v ňom môže siahnuť na sieť
  (`url()`, `@import`, `image-set()`). Zdroj diagramu pochádza zo súborov projektu, teda aj
  z cudzieho repozitára. Preto náhľad odmietne s jasnou správou: init direktívy
  `%%{…}%%`, `config:` v úvodnej hlavičke (`title:` je v poriadku) a príkazy `style`,
  `classDef`, `linkStyle` a C4 `Update…Style` s čímkoľvek iným než obyčajnými hodnotami
  (názvy, `#hex`, čísla, `rgb()`/`hsl()`): žiadne `url(`, `\` ani `@`. Mermaid navyše
  dostane `secure` kľúče, takže direktívy nemôžu meniť `themeCSS`, `themeVariables` ani
  písma. AI prompty takéto konštrukcie nepoužívajú.
- **Vykresľuje sa sériovo**: Mermaid nie je znovu-vstupný (viacero súbežných vykreslení by
  si mohlo prekážať), preto `mermaid.ts` udržiava frontu a spracúva vykreslenia jedno po
  druhom, s unikátnym id pre každé. Dočasný DOM uzol, ktorý si Mermaid necháva počas
  vykreslenia (a pri chybe parsovania sám nevymaže), sa vždy upratuje.
- **Neplatný zdroj**: chyba parsovania sa skráti na prvý čitateľný riadok
  (`mermaidErrorMessage()`) a zobrazí sa spolu so zdrojovým kódom diagramu — náhľad nikdy
  nezhavaruje, prepínač Diagram/Code funguje ďalej.
- **Zastaraný výsledok nikdy nevyhrá**: každá zmena obsahu zvýši interné poradové číslo
  (`request`); keď asynchrónne vykreslenie dorazí neskoro (napr. používateľ medzitým
  upravil diagram alebo prijal AI zmenu), jeho výsledok sa zahodí, ak číslo už nesedí —
  rovnaký vzor, aký `PlantUMLPreview.svelte` používa pri lokálnom PlantUML vykreslení.
- **Limit**: rovnakých 256 KiB ako PlantUML (`MAX_MERMAID_SOURCE_BYTES` v `mermaid.ts`),
  kontrolované ešte pred načítaním balíka Mermaid.

Náhľad je v oboch prípadoch rovnaký komponent (`PlantUMLPreview.svelte`); líši sa len
popisok panela — "PlantUML Diagram" / "Mermaid Diagram" — a to, že PlantUML čaká na
kliknutie, kým Mermaid sa vykreslí sám.

## Export do HTML

`Toolbar.svelte` (`exportHTML`) a `src/lib/export.ts` (`buildHtml`) vykresľujú do
exportovanej stránky oba formáty rovnako: PlantUML cez `render_local_diagram`, Mermaid cez
`renderMermaidSvg()`, výsledok cez `localSvgUrl()` vložený ako `<figure><img
src="data:image/svg+xml…">`. Diagram, ktorý sa nepodarí vykresliť (nepodporovaná syntax,
chyba lokálneho rendereru), zostane v exporte ako zdrojový kód v `<pre><code>` — export
nikdy nezlyhá kvôli jednému diagramu. Stavová správa po uložení hovorí, koľko diagramov
takto zostalo ako zdroj ("N diagram(s) included as source (not renderable locally)").

## Čo v tomto kroku nie je

- Editácia zdroja diagramu priamo v náhľade (zdroj sa dnes mení len cez AI generovanie
  alebo AI rework konkrétneho diagramu).
- Mermaid v lokálnom renderer pre Visio/EA export (tie zostávajú neimplementované
  placeholdery, pozri `docs/ZLEPSENIA.md`).
- Plná vizuálna vernosť Mermaid predlohy (farebné témy, vlastné CSS) — sanitizácia ju
  zámerne obmedzuje na farby, čiary, písmo a lokálne odkazy (šípky); text a tvary
  zostávajú čitateľné.
- Mermaid témy a direktívy (`%%{init: …}%%`) — lokálny náhľad ich odmieta, pozri vyššie.
