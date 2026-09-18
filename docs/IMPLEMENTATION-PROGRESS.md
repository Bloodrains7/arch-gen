# Implementácia roadmapy

## E0, prvý prírastok — projekty a ochrana asynchrónnych výsledkov

Implementované 2026-09-05:

- Nový projekt, názov projektu, Open project, Save project a Save as. `.archgen` je SQLite súbor s application ID a verziou formátu 1. Ukladá dokumenty, aktívny dokument, šablónu, jazyk, sekcie, diagramy a ich ID.
- Rust validuje štruktúru, unikátne ID, revízie a maximálny obsah 32 MiB. Uloženie je transakcia s kontrolou poslednej uloženej revízie; súbežný writer sa odmietne. Save as vytvára nový súbor, neprepisuje existujúci. Cudzí alebo nepodporovaný SQLite súbor sa odmietne.
- Pri každom explicitnom uložení vznikne nemenný checkpoint v tabuľke revisions. Identické opakované uloženie nevytvára ďalší checkpoint.
- Stabilné ID dokumentov/sekcií/diagramov a revízia dokumentu. AI a načítanie šablóny zachytávajú pôvodný dokument, revíziu a reláciu otvoreného projektu. Prepnutie tabu nepresmeruje výsledok. Zmenený alebo odstránený cieľ vedie k obnoviteľnému výsledku s akciami Recover as new document / Discard result.
- Výber konkrétneho diagramu pri AI úprave; pri viacerých diagramoch sa bez výberu negeneruje. Zachováva sa ID upravovaného diagramu a ostatné diagramy.
- Upozornenie na neuloženú prácu pri otvorení/novom projekte/zavretí aplikácie; chybný import ponechá aktuálny projekt. Dokumentové taby majú klávesové ovládanie.
- Sanitizácia Markdownu cez DOMPurify. Inline médiá v Markdown preview sa nezobrazujú, aby sa pri importe neodosielali automatické vzdialené požiadavky. Verejný PlantUML renderer sa spúšťa až explicitnou akciou pri konkrétnom diagrame; po zmene obsahu treba akciu zopakovať.
- Opravené pôvodné tri typové chyby; Monaco pri programovej synchronizácii nehlási zmenu ako používateľskú editáciu.

## Použitie

Spusti desktopovú aplikáciu cez `npm run tauri dev`. Hore pomenuj projekt a použi Save project na nový súbor `.archgen`. Po reštarte ho načítaj cez Open project. Markdown/HTML exporty zostávajú v pôvodnej lište a nenahrádzajú uloženie projektu. Ak je súbor upravený inou inštanciou, Save as zachová aktuálnu verziu do nového súboru.

Samotný `npm run dev` spúšťa frontend, nie Rust backend; natívne ukladanie a dialógy vyžadujú Tauri.

## Overenie

- `npm test`: doménové regresie pre roundtrip, revízie a asynchrónne ciele.
- `cargo test --manifest-path src-tauri/Cargo.toml --offline`: reálne SQLite roundtrip/checkpointy, odmietnutie starého writeru, cudzieho súboru a neplatných ID.
- `npm run test:e2e`: headless Chromium, UI tok s mockovaným Tauri IPC. Nepredstiera overenie natívneho súborového dialógu ani živého LLM.
- `npm run check`, `npm run build`: typy a produkčný frontend.

## E0, druhý prírastok — Undo/Redo, uložená história a AI náhľad

Implementované 2026-09-05:

- Undo/Redo pre aktuálnu reláciu: ručné úpravy sekcií, názvu projektu/dokumentov, jazyka, šablóny, pridanie/odstránenie dokumentu, prijatie AI výsledku a obnovenie historickej verzie. Písanie do jedného poľa sa zoskupuje s medzerou do 750 ms. Prepnutie tabu nie je samostatný undo krok.
- Každé Undo/Redo a obnovenie verzie zvyšuje pracovnú revíziu. Obnovené dokumenty dostanú novú revíziu, takže starý AI výsledok sa nemôže znovu stať platným iba preto, že obsah vyzerá rovnako.
- Saved history číta SQLite checkpointy po 50 položkách a funguje aj po reštarte. Náhľad zobrazuje názov projektu, dokumenty, šablóny, jazyk, text a diagramové zdroje. Restore this version obnoví celý projekt do novej pracovnej revízie. Pôvodné checkpointy sa nemenia a aktuálny rozpracovaný obsah možno vrátiť cez Undo. Novú verziu treba explicitne uložiť.
- Rust poskytuje read-only list_project_history a load_project_revision. Overuje príslušnosť histórie k projektu aj zhodu ID/revízie v SQLite riadku a jeho payload-e. Formát 1 zostáva kompatibilný, migrácia existujúcich súborov nie je potrebná.
- Každý AI výsledok sa odloží na kontrolu. Review changes ukáže pôvodné a navrhované zdroje vedľa seba, počty pridaných/odstránených/zmenených sekcií a nové poradie. Nezmenené sekcie možno rozbaliť. Prijatie znovu overí projekt, reláciu a revíziu dokumentu; pri konflikte je blokované a zostáva obnova do nového dokumentu.
- Discard result nemení dokument. Accept changes je samostatný Undo krok. Načítanie statickej šablóny zostáva priamou používateľskou akciou a je vratné cez Undo.
- Opravená synchronizácia editovateľného nadpisu po Undo/Redo: test kontroluje skutočné dáta aj viditeľný nadpis.

Undo história je pamäťová, najviac 50 krokov, s približným 32 MiB rozpočtom serializovaných snapshotov na zásobník (aspoň jeden snapshot sa zachová). Po reštarte je dostupná história explicitne uložených checkpointov, nie všetkých stlačení klávesov. Aktuálny prírastok prijíma AI návrh ako celok; čiastočné prijatie po blokoch a automatický merge zostávajú ďalším rozšírením.

## Paralelný prírastok — konfigurácia lokálneho AI providera

- Python engine už pri importe nehľadá modely a nevykonáva testovacie generovanie. Pred importom DSPy používa pribalené cenové metadáta LiteLLM namiesto vzdialeného sťahovania.
- Model a timeout sa nastavujú explicitnými premennými prostredia. Predvolený model je pevne `llama3.2:3b`; automatická voľba iného modelu bola odstránená.
- Každý generovací job vytvorí vlastný LM a DSPy context. Zdieľané generátory sú chránené serializáciou jobov. Timeout, nedostupnosť a chýbajúci model vracajú zrozumiteľné chyby bez surových promptov/odpovedí v chybovej správe.
- `health_check()` umožňuje explicitne overiť metadáta modelu bez inferencie. Rust-visible podpisy `run_agent` a `run_doc_agent` zostávajú zachované.
- 11 offline testov pokrýva konfiguráciu, neprítomnosť import-time volaní, chyby, context a súbežné volania. Doplnkový smoke test použil reálne nainštalované DSPy/LangGraph s mockovaným LM/cache; živé Ollama volanie nebolo vykonané.

Konfigurácia a limity: [AI-PROVIDER.md](AI-PROVIDER.md). Ide zatiaľ iba o lokálnu Ollama cez loopback, nastavenie cez prostredie a timeout na jednotlivé volanie. UI nastavenia, celkový deadline jobu, cancellation, schopnosti providera a schema validation zostávajú ďalším krokom.

## E0, tretí prírastok — Rust editovacie transakcie a trvalé AI úlohy

Implementované 2026-09-05:

- Editovacia relácia má autoritatívny snapshot v Rust/SQLite. Frontend zobrazuje optimistické úpravy a posiela ich sériovo; Rust kontroluje očakávanú revíziu, identitu projektu a validitu obsahu a sám prideľuje pracovné revízie. Revízie dodané klientom pri editácii ignoruje. Save počká na potvrdené úpravy. Chyba zastaví frontu, zachová viditeľný obsah a ponúkne Retry synchronization.
- AI dokumentácia aj diagramy používajú trvalé úlohy s pôvodným dokumentom a presným cieľovým ID. Rust zostaví návrh; jeho prijatie alebo obnova do nového dokumentu mení projekt aj stav úlohy v jednej transakcii. Prijatie kontroluje celý pôvodný dokument vrátane revízie. Undo/Restore invaliduje aj obsahovo identický starý návrh.
- Register uchováva queued/running/ready/failed/interrupted/cancelled aj konečné stavy. Po spustení aplikácie sa rozpracované úlohy označia interrupted, bez automatického opakovania. Hotové návrhy sa po otvorení príslušného uloženého projektu znova zobrazia na kontrolu; pri staršej verzii dokumentu zostáva Recover as new document.
- Cancel generation je logické zrušenie: neskorý výsledok už nemôže obnoviť úlohu ani zmeniť dokument. Samotná inferencia sa nemusí okamžite zastaviť. Zlyhané/prerušené návrhy možno odstrániť z aktívneho zoznamu cez Dismiss job.
- Lokálny register je `runtime.sqlite3` v Tauri app-data adresári, oddelený od `.archgen`. Obsahuje prompty, zachytené dokumenty, výsledky a posledné snapshoty relácií v čitateľnom SQLite formáte. Nie je súčasťou exportu ani prenosu projektového súboru. Zatiaľ nemá automatické čistenie; zoznam vracia všetky nevyriešené úlohy a posledných 50 konečných. Na návrat k projektu po reštarte ho treba explicitne uložiť a otvoriť; automatická obnova neuložených relácií ešte nemá UI.

Overenie tohto prírastku: 15 Rust testov (reálne SQLite transakcie a reštart registra), 14 JS testov (vrátane serializácie editov a zastavenia pri chybe), 13 UI testov s mockovaným IPC (vrátane obnovy návrhu po reload/open, zrušenia s neskorým výsledkom a opakovania zlyhanej synchronizácie). Typová kontrola má 0 chýb a 5 existujúcich CSS upozornení; produkčný build prešiel. Natívny desktopový tok so živým LLM nebol v tomto prírastku overený.

## E0, štvrtý prírastok — lokálny render a distribučný spike

- Render locally (no upload): privátny PlantUML/JRE, SVG cez Rust bez verejného servera, bez automatického fallbacku. Sanitizácia SVG, časové a veľkostné limity, SANDBOX profil, blokovanie preprocessor/include direktív a Smetana bez externého Graphviz. Skutočne overený sekvenčný aj class diagram a syntaktická chyba.
- Reprodukovateľný setup s pevnými URL/SHA-256; Tauri resource konfigurácia pre privátny renderer. V tomto checkout-e je runtime pripravený. Nový checkout vyžaduje `scripts/setup-renderer.ps1` pred buildom.
- Odstránené osobné Python cesty z Cargo konfigurácie/build skriptu. Read-only Python probe overí architektúru, DLL a inventár balíkov bez AI volaní. Python balík, dependency lock a test čistého Windows zostávajú otvorené; nejde o hotový prenosný inštalátor.
- Overenie: 18 Rust testov vrátane reálnej Javy, 14 JS a 15 UI testov; typová kontrola bez chýb (5 existujúcich CSS warnings), frontend build prešiel. Podrobný rozsah, obmedzenia a release brány: [RUNTIME-DISTRIBUTION.md](RUNTIME-DISTRIBUTION.md).

## E0, piaty prírastok — izolovaný Python a portable release

Implementované 2026-09-06:

- Privátny CPython 3.14.7 x64 z hash-overeného embeddable archívu; 84 tranzitívnych wheel závislostí uzamknutých verziou a SHA-256. Vývojové site-packages sa nekopírujú ani nemenia. NumPy v balíku aktualizované na ne-yanked patch 2.4.6.
- Setup vytvára nový izolovaný stage a podporuje offline rebuild z cache. Runtime používa DLL-adjacent `_pth`, bez používateľských import paths, registry a vykonávania `.pth` súborov. Bytecode sa nezapisuje do application resources; Python engine má explicitnú cestu, release bez cwd fallbacku.
- Portable release obsahuje Python DLL vedľa executable, engine, privátny JRE/PlantUML a vložený frontend cez `custom-protocol`. Packaging odmieta debug/dev-server variant. Manifest zaznamenáva verzie, licenčné metadáta, SHA-256 a smoke výsledok.
- Skutočný release executable prešiel `--offline-runtime-check` s minimálnym PATH a neplatným PYTHONHOME/PYTHONPATH: načítaná vlastná DLL, DSPy/LangGraph/native moduly, validná dependency closure a lokálny render. Inference je v teste mockovaná; nejde o živú Ollama skúšku ani čistý Windows VM.
- Overenie: 20 Rust, 11 Python provider a 3 packaging testy; offline rebuild aj release smoke prešli. Balík: `portable-builds/archgen-win-x64-he8r3esf/` (~391,5 MB). Recept a otvorené release podmienky: [RUNTIME-DISTRIBUTION.md](RUNTIME-DISTRIBUTION.md).

## Čo ešte nie je dokončené z E0

Nejde o dokončenie celej etapy. Ukladanie je explicitné, bez autosave a automatického otvorenia posledného projektu. Checkpointy nezachytávajú každú editáciu. Neprijaté AI výsledky zostávajú v lokálnom registri, nie v prenosnom `.archgen` súbore.

ID manuálne vytvorených dokumentov/sekcií vznikajú vo frontendovej doménovej vrstve a Rust ich validuje; ID relácií, úloh a AI výstupov vytvára Rust. Pracovné revízie sú autoritou Rustu. Editácia zatiaľ posiela celý navrhovaný snapshot, nie samostatný typovaný príkaz pre každé pole. ReadSet pokrýva pôvodný dokument (vrátane jazyka a šablóny); externé snapshoty/pravidlá ešte neexistujú.

Zrušenie výpočtu, plná kompatibilita lokálneho rendereru (C4 makrá, Mermaid a ďalšie syntaxe), CSP, podpísaný inštalátor/licenčný audit, kompletné rozhranie schopností/provider schém a oprava placeholder exportov zostávajú otvorené. Čistá inštalácia na Windows ani živé AI generovanie zatiaľ nie sú overené. Vývojový build stále potrebuje nainštalovaný Python, vyberaný cez PATH alebo `PYO3_PYTHON`; portable release má vlastný interpreter.

Ďalší odporúčaný krok: čistý Windows smoke test a živá Ollama skúška; následne základ zdrojových snapshotov pred importom repozitára/UI/DB.
