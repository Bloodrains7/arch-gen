# AI prepracovanie označených blokov

Označíš sekcie alebo diagramy, napíšeš inštrukciu a vybraný AI provider vráti návrh.
Návrh sa nikdy nepoužije sám: prejde cez **Review changes** a až **Accept changes** ho
zapíše do dokumentu. Všetko mimo označenia zostáva bajt po bajte rovnaké.

Pôvodné generovanie **Docs / Diagram / Auto** sa nezmenilo: beží cez Python engine a lokálnu
Ollamu podľa [AI-PROVIDER.md](AI-PROVIDER.md). Nastavenia z dialógu **AI settings** platia iba
pre **Rework**.

## Použitie

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

## Provideri

| Provider | Kam ide obsah | Čo treba |
|---|---|---|
| Ollama (local) | zostáva na tomto počítači | bežiaca Ollama a nainštalovaný lokálny model |
| OpenAI API, Gemini API, Claude API | OpenAI / Google / Anthropic | API kľúč |
| Claude CLI, Codex CLI, Gemini CLI | Anthropic / OpenAI / Google | nainštalované CLI, prihlásené vlastným účtom |

Nastavuje sa v **AI settings** (tlačidlo v paneli v režime Rework): provider, model, Ollama URL,
timeout, API kľúč a **Test provider**, ktorý pošle jednu malú skutočnú požiadavku.

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
- Úloha si pamätá providera a model, ktoré si videl pri odoslaní. Ak sa nastavenia medzitým zmenia,
  úloha zlyhá namiesto toho, aby obsah poslala inam.
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
  na označené bloky a nástroje sú vypnuté.

## Limity

- Najviac 24 000 znakov upravovaného textu na požiadavku (120 000 vrátane kontextu); väčší výber sa
  odmietne pred odoslaním, nie po zaplatenej odpovedi.
- Formát diagramu môže byť iba `plantuml` alebo `mermaid`.
- Druhá spustená inštancia ArchGen označí pri štarte bežiace úlohy prvej ako prerušené (spoločný
  lokálny register úloh). Rozpracované AI úlohy preto spúšťaj v jednej inštancii.
- Docs/Diagram zatiaľ providera z AI settings nepoužívajú.

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
