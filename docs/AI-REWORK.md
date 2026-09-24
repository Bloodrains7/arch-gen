# AI: prepracovanie, dokumentácia a diagramy

Štyri režimy v paneli dole — **Rework**, **Docs**, **Diagram** a **Auto** — teraz idú cez
**ten istý** nakonfigurovaný AI provider z dialógu **AI settings**: rovnaké súhlasné hlásenie
pred odoslaním, rovnaký záznam providera a modelu v úlohe, rovnaká kontrola, že sa nastavenia
medzičasom nezmenili, rovnaké zrušenie a rovnaký review-flow (**Review changes → Accept /
Recover / Discard**). Návrh modelu sa nikdy nepoužije sám: až **Accept changes** ho zapíše do
dokumentu.

Python engine (lokálna Ollama, pozri [AI-PROVIDER.md](AI-PROVIDER.md)) generovanie už nepoháňa —
zostáva len ako súčasť offline kontroly v portable builde. Nastavenia `ARCHGEN_AI_*` z
AI-PROVIDER.md preto od tejto zmeny na Docs/Diagram/Auto/Rework nemajú žiadny vplyv.

## Rework: označené bloky

1. V canvase zaškrtni sekcie (checkbox v hlavičke sekcie) alebo jednotlivé diagramy. Panel dole sa
   prepne do režimu **Rework** a ukáže, čo je označené, ktorý provider odpovie a komu obsah pôjde.
2. Napíš inštrukciu („prepíš formálnejšie", „pridaj do diagramu Redis cache") a stlač **Generate**.
3. **Review changes** ukáže pôvodný a navrhovaný zdroj vedľa seba, zhrnutie od modelu a poznámky
   ArchGen (napr. „Ignored changes to 1 block(s) that were not selected.", „Removes diagram …").
4. **Accept changes**, **Recover as new document** alebo **Discard result**. Prijatie je krok Undo.

**Celý dokument** sa prepracuje iba vedome: klikni **Rework**, keď nie je nič označené. Táto voľba
platí len pre aktuálny dokument a jednu požiadavku; zruší ju každé zaškrtnutie, prepnutie tabu aj
odoslanie. Ak je výber prázdny a celý dokument nie je zvolený, Generate aj Enter sú zablokované.

**Include the rest of the document as read-only context** pošle aj neoznačené bloky, aby model
videl súvislosti. Meniť ich nemôže. Voľba sa po každej požiadavke vypne.

## Docs, Diagram, Auto

**Docs** vygeneruje obsah pre štruktúru sekcií, ktorú si zvolil v bočnom paneli: pomenovaná šablóna
(arc42, C4, TOGAF, …) pošle svoje vlastné názvy sekcií a krátky pokyn ku každej („guidance"); pri
šablóne **Custom** alebo neznámej šablóne sa pošlú názvy sekcií, ktoré má dokument práve teraz (bez
pokynu), prípadne jedna sekcia „Overview", ak dokument ešte žiadnu nemá. Model vráti obsah pre presne
tieto sekcie v tomto poradí — vlastný názov, ktorý prípadne vráti, sa zahodí; ArchGen nikdy neponechá
menej ani viac sekcií, než bolo požadovaných. **Diagram** vygeneruje alebo upraví jeden diagram: pri
úprave existujúceho diagramu ArchGen pošle jeho aktuálny obsah aj formát ako referenciu a **formát sa
vždy zachová** bez ohľadu na to, čo model vráti; pri novom diagrame si formát (`plantuml`/`mermaid`)
volí model. **Auto** rozpozná podľa textu promptu, či ide o dokumentáciu alebo diagram.

Panel ukazuje providera a súhlasné hlásenie v každom z týchto troch režimov rovnako ako pri Rework —
napr. „Sends the description and the template's section titles and guidance to Anthropic via Claude
API." pre Docs, alebo pre update diagramu aj s odkazom na existujúci obsah. Pri lokálnej Ollame sa
nezobrazuje nič (nič neopúšťa tento počítač). Ak provider nie je nastavený alebo nie je dostupný,
Generate aj Enter sú zablokované s dôvodom — presne ako pri Rework.

**Pretiahnutie diagramovej dlaždice** z bočného panela na sekciu (alebo na prázdny canvas) spustí
generovanie diagramu okamžite, bez textového poľa s inštrukciou. Preto sa tu súhlas pýta cez
systémový potvrdzovací dialóg: pri inom než lokálnom provideri sa objaví otázka „Sends this diagram's
description to {recipient} via {label}. Continue?" a pri zrušení sa neodošle nič. Lokálna Ollama sa
nepýta.

## Provideri

| Provider | Kam ide obsah | Čo treba |
|---|---|---|
| Ollama (local) | zostáva na tomto počítači | bežiaca Ollama a nainštalovaný lokálny model |
| OpenAI API, Gemini API, Claude API | OpenAI / Google / Anthropic | API kľúč |
| Claude CLI, Codex CLI, Gemini CLI | Anthropic / OpenAI / Google | nainštalované CLI, prihlásené vlastným účtom |

Nastavuje sa v **AI settings** (tlačidlo v paneli, viditeľné v každom zo štyroch režimov): provider,
model, Ollama URL, timeout, API kľúč a **Test provider**, ktorý pošle jednu malú skutočnú požiadavku.

- CLI vždy použije **svoje prihlásenie** (predplatné), nikdy API kľúč z prostredia — ArchGen kľúče
  z prostredia spúšťaným procesom odoberá.
- Ollama: povolené sú iba nainštalované lokálne modely. Modely, ktoré Ollama iba preposiela na
  ollama.com (`…:cloud`, `…-cloud` alebo so vzdialeným hostiteľom), sa odmietnu aj pod iným menom.
- Ollama URL musí byť loopback (`localhost`, `127.0.0.1`, `[::1]`).

## Súkromie

- Mimo lokálnej Ollamy stojí pri tlačidle Generate veta, komu obsah pôjde, ešte pred odoslaním:
  „Sends the selected content to OpenAI via Codex CLI." Pri celom dokumente a pri kontexte to veta
  povie výslovne.
- Posiela sa **iba označené**: bloky, názov dokumentu, šablóna, jazyk a pri diagrame názov jeho
  sekcie. Zvyšok iba so zaškrtnutým kontextom.
- Docs pošle popis, názov dokumentu, šablónu, jazyk a požadované názvy sekcií s pokynmi — nikdy
  existujúci obsah dokumentu (Docs nahrádza obsah cieľových sekcií celý, nie je to
  prepracovanie). Diagram pošle popis, názov dokumentu, šablónu, jazyk a typ diagramu; pri úprave
  existujúceho diagramu k tomu pridá jeho aktuálny obsah a formát ako referenciu — nový diagram
  nič z existujúceho obsahu dokumentu neposiela.
- Úloha si pamätá providera a model, ktoré si videl pri odoslaní. Ak sa nastavenia medzitým zmenia,
  úloha zlyhá namiesto toho, aby obsah poslala inam. Platí pre všetky štyri režimy.
- API kľúče: premenná prostredia (`OPENAI_API_KEY`, `GEMINI_API_KEY`/`GOOGLE_API_KEY`,
  `ANTHROPIC_API_KEY`) alebo dialóg. Uložený kľúč je chránený Windows DPAPI v
  `%APPDATA%\com.smartdawn.archgen\ai-settings.json` — mimo priečinka projektu, teda mimo Gitu.
  Kľúč sa nikdy nevracia do UI, nejde do príkazového riadku ani do súboru a z chýb sa vymazáva.
- HTTP provideri idú cez systémový `curl.exe`; telo požiadavky aj kľúč sú iba na jeho stdin.
- CLI bežia v prázdnom dočasnom priečinku s vypnutými nástrojmi: Claude `--tools "" --strict-mcp-config`,
  Codex `-s read-only --ignore-user-config -c web_search=disabled` + `--disable` pre všetky
  nástrojové funkcie, Gemini s politikou „deny all" a bez MCP serverov. Ak nainštalovaná verzia CLI
  niektorý bezpečnostný prepínač nepozná, provider sa odmietne vetou, nespustí sa slabšie.
- Gemini CLI si ukladá každý prompt (`~/.gemini/tmp`, `~/.gemini/history`) a pri chybe API aj do
  dočasného priečinka. ArchGen mu dáva vlastný dočasný priečinok a po každej požiadavke tieto stopy
  maže; po páde ich uprace pri ďalšom štarte.
- Provider beží vo Windows Job Objecte: Cancel, timeout aj zavretie ArchGen ukončia celý strom
  procesov.
- Obsah dokumentu je pre model iba dáta. Aj keby obsahoval pokyny, odpoveď je len návrh obmedzený
  na označené bloky (Rework), na požadované sekcie (Docs) alebo na jeden diagram (Diagram) a
  nástroje sú vypnuté.

## Limity

- Najviac 24 000 znakov upravovaného textu na požiadavku Rework (120 000 vrátane kontextu); väčší
  výber sa odmietne pred odoslaním, nie po zaplatenej odpovedi.
- Formát diagramu môže byť iba `plantuml` alebo `mermaid`; PlantUML musí byť presne jeden blok
  `@startuml … @enduml`. C4 diagram používa PlantUML štandardnú knižnicu
  `!include <C4/C4_Context>` (alebo `C4_Container`, `C4_Component`, `C4_Dynamic`,
  `C4_Deployment`) — nikdy URL ani súborový include.
- Docs: najviac 40 požadovaných sekcií na požiadavku, názov sekcie do 200 znakov, pokyn ku sekcii
  do 1000 znakov; odpoveď modelu musí mať presne toľko sekcií, koľko bolo požadovaných, inak
  úloha zlyhá.
- Druhá spustená inštancia ArchGen označí pri štarte bežiace úlohy prvej ako prerušené (spoločný
  lokálny register úloh). Rozpracované AI úlohy preto spúšťaj v jednej inštancii.

## Overenie naživo

Mockované e2e testy nevidia chybne pomenovaný IPC argument ani zmenu výstupu CLI. Preto existujú
dva skripty, ktoré ovládajú **skutočnú** desktopovú appku cez WebView2 remote debugging:

```powershell
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=9223"; npm run tauri dev
node scripts/live-rework.mjs "Ollama (local)" 1 qwen3.8:27b   # jedna sekcia, reálny provider
node scripts/live-rework.mjs "Codex CLI" 2                     # míňa trochu kvóty Codexu
node scripts/live-scope.mjs                                    # prázdny výber nič neodošle; celý dokument nič nezmaže
```

Skripty menia providera v tvojich AI nastaveniach. V Ruste sú navyše `#[ignore]` testy
`live_ollama`, `live_claude_cli`, `live_codex_cli`, `live_gemini_cli`, `live_openai`, `live_gemini`,
`live_anthropic`: bez providera prejdú so správou, pri rozbitej integrácii zlyhajú.

Návrh, kontrakt a rozhodnutia z review: [AI-REWORK-DESIGN.md](AI-REWORK-DESIGN.md).
