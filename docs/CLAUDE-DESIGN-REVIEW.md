# Claude CLI: posúdenie návrhu

Dátum: 2026-09-05. Posudzovaná verzia: PRODUCT-DESIGN.md 0.1.
Claude CLI 2.1.261; samostatný beh s nástrojmi Read/Glob/Grep, bez oprávnenia upravovať projekt. Úspešný beh v diagnostickom safe mode; predchádzajúce dva pokusy bez výsledku boli ukončené. Nasleduje nezmenený text finálneho posudku. Je to návrhové posúdenie, nie test funkčnosti aplikácie.

**Verdikt:** Návrh je kvalitný a nadpriemerne poctivý. Má jasné princípy, teda dôkazy, používateľ rozhoduje a nič sa neprepisuje potichu, realistické etapy a otvorené otázky priznané v sekcii 8. Slabinou je, že základná vrstva, teda revízie, pôvod údajov, AI provider a runtime, je opísaná ako požiadavky, nie ako rozhodnutia. Pritom práve tieto rozhodnutia potrebujú prvé štyri tasky backlogu. Odporúčam smer schváliť a pred taskom č. 1 doplniť krátku prílohu technických rozhodnutí k bodom 1 až 4 nižšie.

Overené v kóde: Python engine má napevno Ollama cez DSPy na localhost s temperature 0.7, Cargo používa PyO3 s auto-initialize, závislosť na SQLite zatiaľ chýba a netypované polia sú v piatich komponentoch. Sekcia 2 Ollama ani DSPy nespomína.

1. **Sémantika revízií a histórie nie je definovaná.** Sekcie 4, 7/E0, backlog 1 až 3. Dokument používa revision, baseRevision a „kompatibilnú revíziu“, ale neurčuje, či je revízia projektová monotónna alebo per artefakt, ako sa ukladá história a čo je pravidlo kompatibility. Fix: jedna projektová monotónna revízia, append-only tabuľka revízií s hashom obsahu per artefakt, kompatibilita znamená, že na cieľ medzičasom nikto nezapísal, undo je nová revízia s predchádzajúcim obsahom. Vymenovať, ktoré operácie sú undo-schopné.

2. **Pôvod údajov chýba v dátovom modeli.** Sekcia 4 vs. 3.1, 3.4, 5. Text vyžaduje oddeliť importované fakty, AI interpretácie a poznámky a zakazuje AI meniť importované typy, ale ModelElement nesie len odkazy na zdroje. Chýbajú aj objekty Rule, Pattern, Requirement a explicitné ADR. Fix: pôvod importované/AI/ručné na úrovni vlastnosti, zápis importovaných vlastností povolený iba aplikovaním snapshotu a vynútený v Ruste, doplniť Rule, Pattern, Requirement a ADR ako Decision do tabuľky.

3. **Vrstva AI providera neexistuje.** Sekcie 2, 5, 6, E0. Kód má napevno lokálny model s temperature 0.7, návrh spomína lokálny/vzdialený provider len v sekcii 6 a E0 neoveruje AI integráciu, hoci sekcia 2 priznáva, že overená nebola. Fix: nová podsekcia „AI provider“ s rozhraním, minimálnymi schopnosťami modelu ako JSON schema výstup a veľkosť kontextu, temperature 0 pre štruktúrované výstupy, retry a fallback pri nevalidnom výstupe. Do E0 „Hotovo“ pridať jeden overený end-to-end AI beh.

4. **Runtime závislosti sú na kritickej ceste E0/E1, distribúcia je odložená.** Sekcie 5, 7, 8. PyO3 auto-initialize potrebuje u používateľa Python s balíkmi, lokálny PlantUML potrebuje Javu. Sekcia 8 necháva formu distribúcie otvorenú, ale E0 vyžaduje lokálny render a E1 je prvý produkt. Fix: rozhodnúť pred E0, teda embedded Python s pinned requirements a pre PlantUML pribalené JRE alebo natívnu alternatívu. Do E0 „Hotovo“ pridať čistú inštaláciu na Windows bez predinštalovaného Pythonu a Javy.

5. **Model diffu a prijatia nie je špecifikovaný.** Sekcie 3, 4, 5. Diff s prijať/odmietnuť je centrálna interakcia, ale chýba granularita, čiastočné prijatie a správanie pri súbežnej ručnej úprave. Fix: tri typy diffu, teda textový blok, štruktúrovaná zmena elementu a diagramový variant, prijatie po blokoch, pri zmene bázy trojcestný merge textu alebo konflikt podľa sekcie 4.

6. **Metamodel komponentov a jeho editor chýbajú.** Sekcie 3, 3.4, 4, E2. E2 vyžaduje manuálny model komponentov a oponent nad ním pracuje, ale sekcia 3 nemá pre modelovanie podsekciu a druhy elementov a vzťahov nie sú vymenované. Fix: doplniť 3.7 „Model komponentov“ s malým pevným metamodelom, teda komponent, rozhranie, dátový sklad, externý systém, závislosť, tok, a formulárovým editorom. Úrovne v štýle C4 uviesť ako predpoklad na potvrdenie.

7. **Osud existujúceho generovania z textu nie je určený.** Sekcie 2, 3, 7. PromptPanel a Canvas generujú z voľného textu, no žiadna etapa nehovorí, či sa to zachová, premapuje na projektový model alebo odstráni. E1 tiež neuvádza, či obsahuje nejakú AI funkciu. Fix: v E0 explicitne „voľný opis je ručný zdroj typu Note“ s rovnakým job a diff mechanizmom, v E1 vymenovať AI funkcie alebo uviesť „bez AI“.

8. **Stratégia pre veľké schémy chýba.** Sekcie 3.1, E1, 8. Kritérium E1 je päť tabuliek a veľkosť cieľových projektov je nepotvrdená. Nie je riešené delenie ERD na kapitoly, trvalé uloženie výberu tabuliek pre reimport ani limity introspekcie. Fix: výber tabuliek ako trvalá súčasť Source, ERD per kapitola alebo schéma s limitom prvkov a odkazmi na externé tabuľky. Do E1 pridať fixture s aspoň 150 tabuľkami a časový limit importu.

## Zapracovanie do verzie 0.2

1. Revízie: prijaté; príloha A definuje monotónnu projektovú revíziu, históriu a undo. Kontrola kompatibility navyše zahŕňa čítané zdroje a pravidlá, nielen cieľ.
2. Pôvod údajov: prijaté; vlastnosti majú samostatný pôvod a Rust vynucuje pravidlá zápisu. Doplnené Requirement, Rule, PatternVersion a ADR.
3. AI provider: prijaté s úpravou; capability negotiation, validácia a obmedzená oprava výstupu. Natívne JSON schema nie je povinné pre každý lokálny model a temperature 0 sa používa iba ak ju model podporuje; nie je zárukou deterministického výstupu. Žiadny automatický prechod k vzdialenému providerovi.
4. Runtime: prijaté riziko, upravený postup; distribučný spike začína v E0, čistý Windows test je brána vydania E1. Konkrétny bundle musí preukázať kompatibilitu a licencie, nesľubujeme vopred fungujúci embedded Python.
5. Diff: prijaté; definované tri typy a konzervatívne konflikty. Automatický trojcestný merge odložený, aby nezväčšoval MVP.
6. Metamodel: prijaté; malý pevný model a formulárový editor, bez záväzku implementovať celý C4/UML štandard.
7. Textové generovanie: prijaté; zachované ako ručný zdroj Note, AI textové vysvetlenia v E1 sú voliteľné a oddelené od faktov.
8. Veľké schémy: prijaté; uložený výber, delenie pohľadov a merateľný benchmark s 150 tabuľkami.

Poznámka k presnosti posudku: pôvodná sekcia 2 už DSPy uvádzala pri validácii, chýbala však explicitná konfigurácia Ollama. Tá bola doplnená. Verziu 0.2 upravil hlavný agent podľa posudku; Claude ju druhýkrát neposudzoval.

