# ArchGen: návrh rozšírenia produktu

Verzia: 0.2 — upravené po posúdení verzie 0.1 cez Claude CLI, 2026-09-05.
Posudok a vyhodnotenie pripomienok: [CLAUDE-DESIGN-REVIEW.md](CLAUDE-DESIGN-REVIEW.md). Verzia 0.2 neprešla druhým posúdením Claude.
Rozsah: produktový a implementačný návrh; tento dokument neznamená implementáciu ani schválenie všetkých technických volieb.

## 1. Cieľ

ArchGen má pomôcť pochopiť existujúci projekt, posúdiť návrhy jeho rozšírenia a vytvoriť konzistentnú dokumentáciu a implementačné zadania. Používateľ určuje rozsah analýzy a prijíma rozhodnutia. AI poskytuje návrhy, námietky a vysvetlenia opreté o identifikovateľné zdroje.

Hlavný postup: projekt → vybrané zdroje → model aktuálneho stavu → návrh zmeny → oponentúra → rozhodnutie → dokumentácia → tasky.

## 2. Východiskový stav v repozitári

- Svelte 5 + Tauri 2, Rust príkazy a Python engine cez PyO3. Zachovať tento základ v prvých etapách.
- `src/App.svelte`: dokumenty v pamäti, bez trvalého projektového modelu. Zápis výsledkov závisí od aktívneho tabu.
- `src/lib/components/PromptPanel.svelte`: generovanie z textu, pri úpravách spájanie diagramov a nepresné zacielenie náhrady.
- `src-tauri/src/commands.rs`: prenos textových sekcií a diagramov bez stabilných ID a zdrojov.
- `python-engine/agent.py`: šablóny v kóde, generovanie textu a diagramov, validácia zo štatistík sekcií. Výsledok validácie sa nepoužíva; ERD merge môže nahradiť existujúce polia výstupom modelu.
- AI je aktuálne pevne naviazaná na lokálnu Ollama cez DSPy, s temperature 0.7 a výberom modelu pri importe modulu. SQLite ani abstrakcia providerov zatiaľ nie sú implementované.
- `PlantUMLPreview.svelte`: verejný renderer; `MarkdownPreview.svelte`: HTML bez sanitizácie.
- Visio/EA sú placeholdery. HTML export vkladá zdrojový text diagramu.
- Predchádzajúca kontrola Svelte: 3 chyby typov a 6 upozornení. Desktopový beh ani AI integrácia neboli overené.

## 3. UX a informačná architektúra

Ľavá navigácia projektu: Prehľad, Zdroje, Model, Návrhy, Dokumentácia, Tasky, Šablóny a nastavenia. Nie všetky položky musia vzniknúť v MVP.

Na pracovnej ploche je vybraný artefakt. Pravý kontextový panel ukazuje zdroje, návrhy AI, rozpory a históriu. Každá AI operácia má viditeľný rozsah: konkrétny projekt, revízia, sekcia alebo diagram. Generovanie ponúkne diff s voľbou prijať/odmietnuť; ručné úpravy sa neprepisujú bez kontroly.

Prvé spustenie: názov projektu → jazyk a profil → výber zdroja alebo ručný opis → náhľad načítaného rozsahu → import. Úspech aj chyba musia uvádzať, čo bolo skutočne načítané, čo preskočené a prečo.

### 3.1 Databázový zdroj

Používateľ zvolí pripojenie alebo SQL DDL súbor, databázu/schému a tabuľky. Pred importom vidí rozsah a pravidlo pre väzby mimo výberu. Externý cieľ FK sa zobrazuje ako odkaz bez automatického importu jeho ďalších stĺpcov.

Načítať presné názvy vrátane schémy, dialektové typy, precision/scale, nullable, default, PK/FK vrátane zložených kľúčov, unique, dostupné check constraints a komentáre. Nepodporované metadáta označiť ako nezistené, nie ako neprítomné. Zachovať pôvodný typ aj normalizovanú reprezentáciu; nevyvodzovať API typ iba z DB typu. Kardinality odvodiť z kľúčov, unikátnosti a nullable s uvedením obmedzení.

Prvý dialekt navrhujeme PostgreSQL; ide o predpoklad, ktorý treba potvrdiť pred implementáciou konektora. SQL DDL import iba parsuje podporovaný dialekt, nikdy nespúšťa SQL. Výstup: dátový slovník a deterministicky vytvorený ERD vo vybranej kapitole. AI môže doplniť obchodný význam ako návrh, nesmie meniť importované typy.

### 3.2 Výber časti webovej aplikácie

Prvá verzia: samostatné okno prehliadača ovládané adaptérom; používateľ sa prihlási sám a zapne výber prvku. Výber uloží URL bez citlivých parametrov, lokátor, čas, screenshot oblasti a filtrovanú DOM/accessibility štruktúru. Náhľad pred odovzdaním AI umožní odstránenie citlivých častí. Pre canvas a neprístupné rámce ponúknuť screenshot s explicitne slabšími závermi.

Výstup: popis obrazovky, polia, viditeľné pravidlá a návrh scenárov. Jeden screenshot nepotvrdzuje celý proces ani backendovú architektúru. Procesný diagram vyžaduje zaznamenané kroky alebo používateľom potvrdený opis. Sieťové záznamy až v neskoršej etape s osobitným zapnutím a filtrovaním tokenov, cookies a payloadov. AI sama neodosiela formuláre s obchodným účinkom.

### 3.3 API a repozitár

OpenAPI import doplní operácie, kontrakty a typy. UI pole ↔ API atribút ↔ DB stĺpec je samostatné mapovanie s dôkazmi a stavom návrh/potvrdené/odmietnuté. Názvová podobnosť nie je dôkaz.

Analýza lokálneho repozitára je ďalšia etapa: používateľ vyberie adresáre, vylúčia sa secrets, závislosti a build výstupy. Odkazy na súbory a symboly obsahujú revíziu alebo hash. Bez pripojeného a overeného kódu task nesmie tvrdiť presné implementačné cesty.

### 3.4 Architektonický oponent

Používateľ vytvorí vetvu návrhu nad konkrétnou revíziou modelu: pridá komponent, vzťah alebo pattern a uvedie problém, ciele a obmedzenia. Aktuálny model sa nemení.

Oponentúra obsahuje prínosy, námietky, dotknuté komponenty, alternatívy, otázky a odkazy na podklady. Každá námietka má kategóriu (porušené pravidlo / doložené riziko / hypotéza), závažnosť a zdôvodnenie. Žiadne vymyslené skóre istoty. Pri chýbajúcom kontexte je prípustné „nedostatok údajov“; AI nemusí vždy nesúhlasiť.

Pattern katalóg je spočiatku malý a kurátorovaný: motivácia, predpoklady, dôsledky, alternatívy, zdroje a verzia. Pravidlo projektu môže označiť pattern za preferovaný, nie univerzálne správny. Externé tvrdenia majú citovaný zdroj; projektové tvrdenia odkaz na konkrétny snapshot.

Používateľ prijme/upraví/odmietne návrh alebo prijme riziko s odôvodnením. Vznikne ADR previazané na návrh. Schválený návrh je cieľový stav, nie dôkaz implementácie; prechod do zisteného aktuálneho stavu vyžaduje neskoršie overenie zdrojmi.

### 3.5 Vizuálny editor projektových šablón

„UX generátor template“ tu znamená vizuálny editor šablón dokumentácie a taskov. Generovanie dizajnu obrazoviek aplikácie nie je súčasťou tohto návrhu; význam treba potvrdiť pred touto etapou.

Editor ponúka zoznam blokov, presúvanie, povinnosť, typ poľa, vysvetlenie, príklad a živý náhľad. Základné bloky: text, tabuľka polí, diagram, rozhodnutie, acceptance criteria, implementačné kroky, zdroje. Import ukážkového dokumentu navrhne šablónu na kontrolu.

Oddeliť štruktúru (validovateľné polia), štýl (jazyk, tón, terminológia, vzorové výstupy) a architektonické pravidlá. Priorita: systémové invarianty integrity → výslovná voľba používateľa v operácii → projektové pravidlá → verzia šablóny → predvolené nastavenia. Výnimka sa zaznamená; AI ju nesmie aplikovať potichu.

Šablóna má stabilné ID blokov, draft a nemenné publikované verzie. Projektový variant odkazuje na konkrétnu základnú verziu a explicitné overrides. Výsledný vyriešený obsah sa uloží s artefaktom, aktualizácia rodiča sa nikdy neprejaví automaticky. Používateľ uvidí diff a môže zvoliť migráciu. Začať formulárovým editorom; úplný drag-and-drop designer až po overení dátového formátu.

### 3.6 Generovanie implementačných taskov

Vstup: konkrétny schválený návrh, ADR alebo používateľom vybraný rozsah dokumentácie. Najprv návrh rozdelenia práce a závislostí, potom jednotlivé tasky podľa projektovej šablóny.

Povinné polia: názov; description (problém, účel, výsledné správanie); implementačné zadanie (rozsah a mimo rozsahu, dotknuté komponenty, kontrakty, dátové zmeny a obmedzenia); overiteľné acceptance criteria; overenie; závislosti; zdroje; otvorené otázky. Pri rizikovej migrácii aj postup migrácie a návratu. Odhad je voliteľný a označený ako odhad.

Task môže byť draft aj s otázkami. Stav ready vyžaduje vyriešenie blokujúcich otázok a potvrdenie používateľom. Graf závislostí nesmie mať cykly. Implementačný brief pre AI exportuje task, relevantné výrezy zdrojov, rozhodnutia a hranice práce; neexportuje prihlasovacie údaje. Najprv Markdown/JSON, integrácie Jira/GitHub až neskôr s náhľadom a explicitnou akciou publikovať.

## 4. Dátový model a konzistencia

Pre MVP použiť jeden lokálny SQLite projektový súbor `.archgen` s verziou schémy a transakciami. Väčšie prílohy môžu byť v sprievodnom adresári s relatívnymi cestami a hashmi; export projektu ich musí zahrnúť. Nie je potrebná graph DB.

| Objekt | Minimálne údaje |
|---|---|
| Project | id, názov, schemaVersion, revision, profil |
| Source | id, kind, rozsah, connectorConfig bez secrets, credentialRef |
| Snapshot | id, sourceId, čas, hash, stav úplnosti, diagnostika |
| ModelElement | id, kind, qualifiedName, vlastnosti, sourceRefs |
| Relationship | id, sourceElementId, targetElementId, kind, vlastnosti, sourceRefs |
| Evidence | snapshotId, cesta/lokátor/DB identifikátor, spôsob získania |
| Mapping | koncové ID, evidence, navrhnuté/potvrdené/odmietnuté |
| Proposal | id, baseRevision, zmeny, ciele, obmedzenia, stav |
| Review/Decision | proposalId, zistenia, evidence, ľudské rozhodnutie |
| TemplateVersion | id, parentVersion, blocks, style, overrides, schema |
| Artifact/Task | id, revision, templateVersion, resolvedTemplate, sourceRefs, obsah, stav |
| Job | id, projectId, targetId, baseRevision, stav, priebeh, chyba |

ID generuje aplikácia, nie LLM. Reimport páruje stabilné zdrojové identifikátory; premenovania bez dôkazu ponúkne na potvrdenie. Chýbajúci objekt v neúplnom importe nesmie znamenať odstránenie.

Oddeliť importované fakty, AI interpretácie a ručné poznámky. Aktualizácia zdroja vytvorí nový snapshot a diff, zachová poznámky a označí závislé artefakty ako zastarané. Regenerácia je návrh novej revízie. Undo/história musia fungovať aj po reštarte.

Každá operácia zachytí projectId, targetId a baseRevision pred spustením. Výsledok sa aplikuje iba na ten istý cieľ a kompatibilnú revíziu; inak sa uloží ako konflikt na porovnanie. Zavretie alebo prepnutie tabu nesmie presmerovať výsledok. Zrušený job nikdy nezapíše výsledok.

## 5. Technická architektúra

- Svelte: typované view modely, projektový store, formuláre šablón, diff a práca s výberom. Nahradiť `any[]` explicitnými kontraktmi.
- Rust: jediný vlastník ukladania a transakcií, ID, revízií, credential references a fronty jobov; Tauri príkazy na projekty, snapshoty a aplikovanie zmien.
- Python: adaptéry introspekcie a AI pipeline, štruktúrované vstupy/výstupy validované Pydantic schémami. Import schémy nevyžaduje spustenie LLM.
- Zachovať PyO3 pre prvé etapy; dlhé operácie vykonávať mimo UI threadu, s timeoutmi a Tauri progress events. Požiadavka na tvrdé zrušenie alebo izoláciu prehliadača môže vyžadovať samostatný worker proces. Pred browser etapou urobiť spike a zvoliť procesový protokol; nemeniť celý backend vopred.
- Kontrakty majú verziu; testovať serializáciu medzi Rust, TS a Python. Zdieľané fixtures a generované typy uprednostniť pred tromi ručne rozchádzajúcimi sa definíciami.
- AI kroky: vybrať relevantné podklady → vygenerovať štruktúrovaný návrh → validovať → zobraziť diff. Rozpočty kontextu a limity rozsahu sú explicitné; prerušená úloha zachová úspešne pripravené drafty.
- ERD a dátový slovník vzniknú z rovnakého modelu deterministicky. AI diagramy podliehajú syntaktickej kontrole a kontrole odkazov na model. Úprava kódu diagramu vytvára custom variant; nie každý PlantUML je spätne mapovateľný na model.
- Lokálny PlantUML renderer bez vzdialených includes a s obmedzeným prístupom k súborom/sieti. Sanitizácia HTML, CSP a filtrovanie URL. Dostupnosť a balenie rendereru overiť na Windows pred vydaním.

## 6. Hranice dôvery a prevádzka

DB účtu prideliť iba potrebné práva, dotazy na metadáta realizovať adaptérom s timeoutmi a rozsahom. LLM nedostane možnosť spúšťať ľubovoľné SQL. Secrets uložiť do systémového úložiska poverení, projekt nesie iba reference. Na inom stroji treba pripojenie znova nastaviť.

Web, komentáre DB, dokumenty a repozitár sú nedôveryhodné podklady, nie inštrukcie. Ich obsah nesmie meniť oprávnenia nástrojov alebo zapnúť ďalšie zdroje. Nástroje sa autorizujú aplikáciou, nie textom modelu. Browser profil je oddelený od osobného profilu, session dáta sa nepribaľujú do exportu.

Projektové nastavenia určujú lokálny/vzdialený AI provider a povolené kategórie podkladov. Pred odoslaním sa ukáže rozsah podľa politiky projektu. Logy štandardne obsahujú ID, časy a diagnostiku bez surového obsahu. Snapshoty/screenshoty sa dajú vymazať; UI vysvetlí dopad na overiteľnosť starších výstupov.

## 7. Etapy a výstupné podmienky

### E0 — stabilný editor a projekty

Ukladanie/obnova, stabilné ID a revízie, história, presné zacielenie, job states a ochrana pred neskorým výsledkom, typové chyby, sanitizácia a lokálny render. Placeholder exporty nesmú hlásiť úspech.

Hotovo: reštart obnoví obsah; prepnutie tabu a súbežná ručná úprava nespôsobia prepis; odmietnutý/zrušený výsledok sa neaplikuje; známe typové chyby odstránené.

### E1 — prvý ucelený produkt: DB → dokumentácia

Jeden dialekt, výber tabuliek, snapshot a model, dátový slovník + ERD, jednoduchá projektová šablóna cez formulár, Markdown/HTML export so skutočnými diagramami. SQL DDL import ako následné rozšírenie v rámci tejto oblasti, nie blokátor živého konektora.

Hotovo: päť vybraných tabuliek dá zhodné typy v slovníku aj ERD; zložené FK, nullable a externé väzby sú správne; neúplný import nemaže model; zmena typu po reimporte označí dotknutý výstup; projekt sa dá znovu otvoriť.

### E2 — rozhodnutia a implementačné tasky

Manuálny model komponentov, návrhy nad revíziou, kontext požiadaviek, oponentúra, ADR, tasky a implementačné briefy. OpenAPI import pre presnejšie kontrakty. Malý pattern katalóg.

Hotovo: schválenie návrhu nezmení zistený stav; námietky majú zdroje alebo označenie hypotézy; tasky majú testovateľné kritériá a acyklické závislosti; blokujúce otázky bránia ready stavu.

### E3 — analýza UI a prepájanie zdrojov

Browser spike, výber oblasti, redakcia, screenshot + DOM, zaznamenané kroky a potvrdenie mapovaní UI/API/DB. Podpora prihlásenia, SPA, rámcov a nedostupného DOM s diagnostikou.

Hotovo: analyzuje sa iba vybraný rozsah; zdroje sa dajú dohľadať; screenshot nevytvára potvrdené DB typy; prihlasovacie údaje nevstúpia do promptov/exportu.

### E4 — rozšírenie

Vizuálny designer šablón, import štýlu z príkladu, migrácie verzií, repo analýza, ďalšie DB dialekty, cielené aktualizácie, publikovanie taskov a reálne Visio/EA exporty podľa dopytu. Viacužívateľská spolupráca a cloud synchronizácia zatiaľ mimo rozsahu.

## 8. Overenie a prvý backlog

Overenie kombinovať: unit testy identity/revízií a typov; integračné fixtures pre DB dialekt a zložené constraints; kontraktové testy Rust/Python/TS; UI scenáre uloženie, prepnutie tabu počas jobu a prijatie diffu. AI hodnotiť na malom kurátorovanom súbore projektov: podložené tvrdenia, zachovanie faktov, relevantné námietky, úplnosť taskov. Nespoliehať sa iba na druhý LLM ako rozhodcu.

Prvé implementačné tasky v poradí:

1. Definovať verzované projektové kontrakty, SQLite migrácie a identity artefaktov.
2. Implementovať uloženie/obnovu a históriu revízií.
3. Zaviesť job registry, adresovanie cieľa a prijatie diffu s kontrolou revízie.
4. Opraviť typy, ošetriť HTML a zaviesť lokálne renderovanie.
5. Implementovať vybraný DB adaptér a testovacie schémy.
6. Pridať výber tabuliek, snapshoty a normalizovaný model so zdrojmi.
7. Generovať ERD a dátový slovník, pridať jednoduchú projektovú šablónu.
8. Doplniť reimport, označenie zastaraných artefaktov a prenosný export projektu.

Pred implementáciou konkrétnych etáp zostáva potvrdiť: prvý DB systém, cieľové projekty a ich veľkosť, význam UX template, lokálny/vzdialený model a požadovaná forma distribúcie. Nejde o prekážku dokončenia tohto návrhu.

## 9. Referencie technických možností

- SQLAlchemy reflection: https://docs.sqlalchemy.org/en/20/core/reflection.html
- Playwright locators a snapshoty: https://playwright.dev/docs/api/class-locator

Výber knižníc je návrh; verzie, licencie a balenie sa overia pri implementačnom spike.

## Príloha A — spresnenia po Claude review

Nasledujúce rozhodnutia spresňujú všeobecné formulácie vyššie a majú pred nimi prednosť. Sú návrhom implementačných kontraktov, nie tvrdením o hotovej implementácii.

### A1. Revízie, história a prijatie výsledkov (E0)

Každá transakcia meniaca projektový obsah zvýši jednu monotónnu Project.revision. Artefakt má lastChangedRevision a hash obsahu. História je append-only zoznam zmien s predchádzajúcim a novým obsahom alebo odkazmi na nemenné verzie; pre MVP uprednostniť plné verzie obsahu pred vlastným delta formátom. Stav priebehu jobu nezvyšuje obsahovú revíziu.

Job zachytí targetId, hash cieľa a readSet: ID/verzie použitých snapshotov, pravidiel, šablón a relevantných rozhodnutí. Kompatibilita znamená nezmenený cieľ aj tento readSet. Zmena nesúvisiacej kapitoly výsledok neblokuje. Pri zmene cieľa vznikne konflikt; pri zmene podkladov sa výsledok označí ako zastaraný a nemožno ho bežným prijatím aplikovať bez regenerácie alebo výslovne zaznamenanej výnimky. Odstránený cieľ ani zrušený job neprijmú zápis. Kontrola a zápis sú jedna Rust/SQLite transakcia.

Undo vytvorí novú revíziu s obnoveným obsahom. Platí pre editáciu/odstránenie artefaktu, prijatie AI návrhu, zmenu mapovania, projektovej šablóny a aplikovanie importu. Undo importu obnoví aktívne odkazy modelu, nezmaže získaný snapshot. Undo nevracia externé publikovanie, poverenia ani výslovné vymazanie citlivých príloh. Zmena šablóny po publikovaní vytvára ďalšiu verziu, nikdy neprepíše publikovanú.

### A2. Pôvod vlastností a doplnenie metamodelu

ModelProperty obsahuje elementId, key, value, origin (imported/ai/manual), evidenceRefs, status a revision. Importovaná hodnota je oddelená od navrhnutého vysvetlenia alebo ručnej anotácie rovnakého poľa. Rust akceptuje zmenu imported iba z validovaného snapshot importu. AI výsledok sa parsuje na obmedzené operácie; nepovažuje sa za voľný zápis do modelu.

Doplnené objekty: Requirement (cieľ, merateľná podmienka, zdroj, stav), Rule (rozsah, záväznosť, verzia, odôvodnenie), PatternVersion (predpoklady, dôsledky, alternatívy, zdroje) a Decision/ADR (proposalId, kontext, alternatívy, rozhodnutie, prijaté riziká, autor potvrdenia). Každé má stabilné ID a revíziu.

Prvý komponentový model v E2 podporuje System, Component, Interface, DataStore a ExternalSystem. Vzťahy: contains, exposes, dependsOn a dataFlow. DB Entity je samostatný typ; mapovanie na komponent ani dátový sklad nie je implicitné. Formulárový editor umožňuje pridať prvok, vybrať konce vzťahu a upraviť vlastnosti, diagram je pohľad na model. Validácia zakáže neexistujúce konce a cyklické contains; cyklické závislosti sa označia na posúdenie, nie automaticky zakážu. Úplná sémantika C4/UML nie je cieľom E2.

### A3. AI provider a existujúce textové generovanie

Rozhranie providera: capabilities(), healthCheck(), generate(request) a normalizované chyby. Konfigurácia explicitne určuje provider, model, kontextový limit, podporu štruktúrovaného výstupu, timeout a povolené kategórie dát. E0 implementuje súčasnú Ollama za týmto rozhraním; vzdialený provider je ďalší adaptér. Inicializácia modulu nesmie spúšťať skúšobné generovanie ani automaticky meniť model.

Preferovať natívny štruktúrovaný výstup, ak ho model podporuje. V ostatných prípadoch parsovať JSON a vždy overiť Pydantic schémou aj doménovými invariantmi. Najviac jeden opravný pokus s validačnými chybami; potom explicitná chyba bez aplikovania výsledku. Pre štruktúrované úlohy nízka temperature alebo 0, iba ak parameter model podporuje; nejde o záruku deterministickosti. Timeout/rate limit má ohraničené opakovanie a podporu zrušenia. Automatické prepnutie na iného providera alebo odoslanie ďalších dát nie je povolené.

Voľný textový opis zostáva zachovaný ako Source typu Note s autorom a revíziou. Nie je overeným zistením o nasadenom systéme. Doterajšie generovanie prejde na rovnaké joby, ID a diffy. E0 overí jeden celý lokálny AI beh a správanie pri neplatnom výstupe a nedostupnom modeli. E1 funguje bez AI pre import, ERD a slovník; voliteľná AI doplní popisy a vysvetlenia ako označené návrhy. Takto vypnutý model neblokuje dokumentáciu presných DB faktov.

### A4. Runtime a distribučná brána

Distribučný spike je prvý technický krok E0, paralelne s definovaním kontraktov. Cieľový predpoklad: Windows x64; aplikácia nemá vyžadovať systémový Python ani Javu. Overí sa bundlovaný kompatibilný Python pre PyO3, uzamknuté Python závislosti a PlantUML s privátnym JRE, prípadne preukázateľne kompatibilný alternatívny renderer. Overiť aj potrebné pomocné renderer nástroje, DLL, licencie a veľkosť balíka.

Ollama a model môžu byť v prvej verzii explicitne externou požiadavkou iba pre AI funkcie, s diagnostikou a návodom pri prvom použití. DB import a lokálny render musia fungovať bez nej. Ak sa PyO3 bundle nedá spoľahlivo zostaviť, samostatný Python worker sa musí rozhodnúť ešte pred implementáciou DB adaptéra; výsledok spike bude samostatné ADR.

E0 môže prebiehať v riadenom vývojovom prostredí. Vydanie E1 blokuje čistý Windows smoke test: inštalácia, otvorenie projektu, import fixture a lokálne SVG bez systémového Pythonu/Javy; potom AI smoke test s deklarovanou Ollama konfiguráciou. Toto je technická brána, nie tvrdenie, že distribúcia je už vyriešená.

### A5. Granularita diffu

Tri jednotky: textový blok so stabilným ID; štruktúrovaná sada operácií nad elementmi/vlastnosťami; celý diagramový variant. Text možno prijať po blokoch, diagram ako celok. Štruktúrované operácie majú explicitné závislosti: prijatie vzťahu vyžaduje existujúce alebo spoločne prijaté koncové prvky. Každá prijatá sada prejde validáciou a transakciou.

Pri súbežnej úprave sa v MVP zobrazí pôvodný obsah, súčasný obsah a návrh na manuálne vyriešenie. Automatický trojcestný merge nie je podmienkou MVP. Pri čiastočnom prijímaní sa báza zvyšných blokov aktualizuje iba o vlastné prijaté zmeny; cudzia zmena zostáva konfliktom. Regenerácia nikdy automaticky neobnoví odmietnutú zmenu.

### A6. Veľké schémy a trvalý rozsah

Source.scope trvalo ukladá vybrané schémy a tabuľky, pravidlo externých FK a filtre; reimport používa tento rozsah. Zmena výberu sa odlišuje od zmazania tabuľky v zdroji. Rozsah je súčasťou hashu snapshotu.

Introspekcia má limity a progress po skupinách tabuliek. Počiatočný návrh: limit dotazu 15 s a celého jobu 120 s, konfigurovateľné a overené benchmarkom. Prekročenie končí neúplným snapshotom s diagnostikou, nikdy tichým skrátením. ERD pohľad má predvolene najviac 30 plných entít; pri prekročení používateľ rozdelí pohľady podľa modulov alebo výberu. Model ani slovník sa tým nestrácajú, externé väzby zostávajú označené.

E1 integračný benchmark: lokálna PostgreSQL fixture 150 tabuliek vrátane zložených FK, zaznamenať hardvér a konfiguráciu; cieľ kompletného načítania metadát do 60 s bez LLM. Overiť výber piatich tabuliek, plný import, timeout aj reimport po zmene výberu. Časový cieľ je návrhový akceptačný limit, nie aktuálne nameraný výkon.
